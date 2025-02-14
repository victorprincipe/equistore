use std::sync::Arc;
use std::ffi::CString;
use std::collections::{HashMap, BTreeSet};

use crate::utils::ConstCString;
use crate::{Labels, LabelsBuilder};
use crate::{eqs_array_t, get_data_origin};
use crate::Error;

/// A `Vec` which can not be modified
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImmutableVec<T>(Vec<T>);

impl<T> std::ops::Deref for ImmutableVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a, T> IntoIterator for &'a ImmutableVec<T> {
    type Item = &'a T;

    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// Single data array with the corresponding metadata inside a `TensorBlock`
#[derive(Debug)]
pub struct BasicBlock {
    pub data: eqs_array_t,
    pub samples: Arc<Labels>,
    pub components: ImmutableVec<Arc<Labels>>,
    pub properties: Arc<Labels>,
}

fn check_data_and_labels(
    context: &str,
    data: &eqs_array_t,
    samples: &Labels,
    components: &[Arc<Labels>],
    properties: &Labels,
) -> Result<(), Error> {
    let shape = data.shape()?;

    if shape.len() != components.len() + 2 {
        return Err(Error::InvalidParameter(format!(
            "{}: the array has {} dimensions, but we have {} separate labels",
            context, shape.len(), components.len() + 2
        )));
    }

    if shape[0] != samples.count() {
        return Err(Error::InvalidParameter(format!(
            "{}: the array shape along axis 0 is {} but we have {} sample labels",
            context, shape[0], samples.count()
        )));
    }

    // ensure that all component labels have different names
    let n_components = components.iter().map(|c| c.names()).collect::<BTreeSet<_>>().len();
    if n_components != components.len() {
        return Err(Error::InvalidParameter(format!(
            "{}: some of the component names appear more than once in component labels",
            context,
        )));
    }

    let mut dimension = 1;
    for component in components {
        if shape[dimension] != component.count() {
            return Err(Error::InvalidParameter(format!(
                "{}: the array shape along axis {} is {} but we have {} entries \
                for the corresponding component",
                context, dimension, shape[dimension], component.count(),
            )));
        }
        dimension += 1;
    }

    if shape[dimension] != properties.count() {
        return Err(Error::InvalidParameter(format!(
            "{}: the array shape along axis {} is {} but we have {} properties labels",
            context, dimension, shape[dimension], properties.count()
        )));
    }

    Ok(())
}

fn check_component_labels(components: &[Arc<Labels>]) -> Result<(), Error> {
    for (i, component) in components.iter().enumerate() {
        if component.size() != 1 {
            return Err(Error::InvalidParameter(format!(
                "component labels must have a single dimension, got {}: [{}] for component {}",
                component.size(), component.names().join(", "), i
            )));
        }
    }
    Ok(())
}

impl BasicBlock {
    /// Create a new `BasicBlock`, validating the shape of data & labels
    pub fn new(
        data: eqs_array_t,
        samples: Arc<Labels>,
        components: Vec<Arc<Labels>>,
        properties: Arc<Labels>,
    ) -> Result<BasicBlock, Error> {
        check_data_and_labels(
            "data and labels don't match", &data, &samples, &components, &properties
        )?;

        check_component_labels(&components)?;
        let components = ImmutableVec(components);
        return Ok(BasicBlock { data, samples, components, properties });
    }

    fn components_to_properties(&mut self, dimensions: &[&str]) -> Result<(), Error> {
        debug_assert!(!dimensions.is_empty());

        let mut component_axis = None;
        for (component_i, component) in self.components.iter().enumerate() {
            if component.names() == dimensions {
                component_axis = Some(component_i);
                break;
            }
        }

        let component_axis = component_axis.ok_or_else(|| Error::InvalidParameter(format!(
            "unable to find [{}] in the components ", dimensions.join(", ")
        )))?;

        let moved_component = self.components.0.remove(component_axis);

        // construct the new property with old properties and the components
        let old_properties = &self.properties;
        let new_property_names = moved_component.names().iter()
            .chain(old_properties.names().iter())
            .copied()
            .collect();

        let mut new_properties_builder = LabelsBuilder::new(new_property_names);
        for new_property in moved_component.iter() {
            for old_property in old_properties.iter() {
                let mut property = new_property.to_vec();
                property.extend_from_slice(old_property);
                new_properties_builder.add(&property)?;
            }
        }
        let new_properties = new_properties_builder.finish();

        let mut new_shape = self.data.shape()?.to_vec();
        let properties_axis = new_shape.len() - 1;
        new_shape[properties_axis] = new_properties.count();
        new_shape.remove(component_axis + 1);

        self.data.swap_axes(component_axis + 1, properties_axis - 1)?;
        self.data.reshape(&new_shape)?;

        self.properties = Arc::new(new_properties);

        Ok(())
    }

    /// Try to copy this `BasicBlock`. This can fail if we are unable to copy
    /// the underlying `eqs_array_t` data array
    pub fn try_clone(&self) -> Result<BasicBlock, Error> {
        let data = self.data.try_clone()?;

        Ok(BasicBlock {
            data,
            samples: Arc::clone(&self.samples),
            components: self.components.clone(),
            properties: Arc::clone(&self.properties),
        })
    }
}

/// A single block in a `TensorMap`, containing both values & optionally
/// gradients of these values w.r.t. any relevant quantity.
#[derive(Debug)]
pub struct TensorBlock {
    values: BasicBlock,
    gradients: HashMap<String, BasicBlock>,
    // all the keys from `self.gradients`, as C-compatible strings
    gradient_parameters: Vec<ConstCString>,
}

impl TensorBlock {
    /// Create a new `TensorBlock` containing the given data, described by the
    /// `samples`, `components`, and `properties` labels. The block is
    /// initialized without any gradients.
    pub fn new(
        data: impl Into<eqs_array_t>,
        samples: Arc<Labels>,
        components: Vec<Arc<Labels>>,
        properties: Arc<Labels>,
    ) -> Result<TensorBlock, Error> {
        Ok(TensorBlock {
            values: BasicBlock::new(data.into(), samples, components, properties)?,
            gradients: HashMap::new(),
            gradient_parameters: Vec::new(),
        })
    }

    /// Try to copy this `TensorBlock`. This can fail if we are unable to copy
    /// one of the underlying `eqs_array_t` data arrays
    pub fn try_clone(&self) -> Result<TensorBlock, Error> {
        let values = self.values.try_clone()?;
        let mut gradients = HashMap::new();
        for (parameter, basic_block) in &self.gradients {
            gradients.insert(parameter.clone(), basic_block.try_clone()?);
        }
        let gradient_parameters = self.gradient_parameters.clone();

        return Ok(TensorBlock {
            values,
            gradients,
            gradient_parameters
        });
    }

    /// Get the values data and metadata in this block
    pub fn values(&self) -> &BasicBlock {
        &self.values
    }

    /// Get read-write access to the values data and metadata in this block
    pub fn values_mut(&mut self) -> &mut BasicBlock {
        &mut self.values
    }

    /// Get all gradients defined in this block
    pub fn gradients(&self) -> &HashMap<String, BasicBlock> {
        &self.gradients
    }

    /// Get the data and metadata for the gradient with respect to the given
    /// parameter in this block, if it exists.
    pub fn gradient(&self, parameter: &str) -> Option<&BasicBlock> {
        self.gradients.get(parameter)
    }

    /// Get the data and metadata for the gradient with respect to the given
    /// parameter in this block, if it exists.
    pub fn gradient_mut(&mut self, parameter: &str) -> Option<&mut BasicBlock> {
        self.gradients.get_mut(parameter)
    }

    /// Get the list of gradients in this block for the C API
    pub fn gradient_parameters_c(&self) -> &[ConstCString] {
        &self.gradient_parameters
    }

    /// Add a gradient with respect to `parameter` to this block.
    ///
    /// The gradient `data` is given as an array, and the samples and components
    /// labels must be provided. The property labels are assumed to match the
    /// ones of the values in this block.
    ///
    /// The components labels must contain at least the same entries as the
    /// value components labels, and can prepend other components labels.
    pub fn add_gradient(
        &mut self,
        parameter: &str,
        data: impl Into<eqs_array_t>,
        samples: Arc<Labels>,
        components: Vec<Arc<Labels>>,
    ) -> Result<(), Error> {
        if self.gradients.contains_key(parameter) {
            return Err(Error::InvalidParameter(format!(
                "gradient with respect to '{}' already exists for this block", parameter
            )))
        }
        let data = data.into();

        if data.origin()? != self.values.data.origin()? {
            return Err(Error::InvalidParameter(format!(
                "the gradient array has a different origin ('{}') than the value array ('{}')",
                get_data_origin(data.origin()?),
                get_data_origin(self.values.data.origin()?),
            )))
        }

        // this is used as a special marker in the C API
        if parameter == "values" {
            return Err(Error::InvalidParameter(
                "can not store gradient with respect to 'values'".into()
            ))
        }

        if samples.size() == 0 {
            return Err(Error::InvalidParameter(
                "gradients samples must have at least one dimension named 'sample', we got none".into()
            ))
        }

        if samples.size() < 1 || samples.names()[0] != "sample" {
            return Err(Error::InvalidParameter(format!(
                "'{}' is not valid for the first dimension in the gradients \
                samples labels. It must must be 'sample'", samples.names()[0])
            ))
        }

        check_component_labels(&components)?;
        if self.values.components.len() > components.len() {
            return Err(Error::InvalidParameter(
                "gradients components should contain at least as many labels \
                as the values components".into()
            ))
        }
        let extra_gradient_components = components.len() - self.values.components.len();
        for (component_i, (gradient_labels, values_labels)) in components.iter()
            .skip(extra_gradient_components)
            .zip(&*self.values.components)
            .enumerate() {
                if gradient_labels != values_labels {
                    return Err(Error::InvalidParameter(format!(
                        "gradients and values components mismatch for values \
                        component {} (the corresponding names are [{}])",
                        component_i, values_labels.names().join(", ")
                    )))
                }
            }

        let properties = Arc::clone(&self.values.properties);
        check_data_and_labels(
            "gradient data and labels don't match", &data, &samples, &components, &properties
        )?;

        let components = ImmutableVec(components);
        self.gradients.insert(parameter.into(), BasicBlock {
            data,
            samples,
            components,
            properties
        });

        let parameter = ConstCString::new(CString::new(parameter.to_owned()).expect("invalid C string"));
        self.gradient_parameters.push(parameter);

        return Ok(())
    }

    pub(crate) fn components_to_properties(&mut self, dimensions: &[&str]) -> Result<(), Error> {
        if dimensions.is_empty() {
            return Ok(());
        }

        self.values.components_to_properties(dimensions)?;
        for gradient in self.gradients.values_mut() {
            gradient.components_to_properties(dimensions)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{LabelValue, LabelsBuilder};
    use crate::data::TestArray;

    use super::*;

    fn example_labels(name: &str, count: usize) -> Arc<Labels> {
        let mut labels = LabelsBuilder::new(vec![name]);
        for i in 0..count {
            labels.add(&[LabelValue::from(i)]).unwrap();
        }
        return Arc::new(labels.finish());
    }

    #[test]
    fn no_components() {
        let samples = example_labels("samples", 4);
        let properties = example_labels("properties", 7);
        let data = TestArray::new(vec![4, 7]);
        let result = TensorBlock::new(data, samples.clone(), Vec::new(), properties.clone());
        assert!(result.is_ok());

        let data = TestArray::new(vec![3, 7]);
        let result = TensorBlock::new(data, samples.clone(), Vec::new(), properties.clone());
        assert_eq!(
            result.unwrap_err().to_string(),
            "invalid parameter: data and labels don't match: the array shape \
            along axis 0 is 3 but we have 4 sample labels"
        );

        let data = TestArray::new(vec![4, 9]);
        let result = TensorBlock::new(data, samples.clone(), Vec::new(), properties.clone());
        assert_eq!(
            result.unwrap_err().to_string(),
            "invalid parameter: data and labels don't match: the array shape \
            along axis 1 is 9 but we have 7 properties labels"
        );

        let data = TestArray::new(vec![4, 1, 7]);
        let result = TensorBlock::new(data, samples, Vec::new(), properties);
        assert_eq!(
            result.unwrap_err().to_string(),
            "invalid parameter: data and labels don't match: the array has \
            3 dimensions, but we have 2 separate labels"
        );
    }

    #[test]
    fn multiple_components() {
        let component_1 = example_labels("component_1", 4);
        let component_2 = example_labels("component_2", 3);

        let samples = example_labels("samples", 3);
        let properties = example_labels("properties", 2);
        let data = TestArray::new(vec![3, 4, 2]);
        let components = vec![Arc::clone(&component_1)];
        let result = TensorBlock::new(data, samples.clone(), components, properties.clone());
        assert!(result.is_ok());

        let data = TestArray::new(vec![3, 4, 3, 2]);
        let components = vec![Arc::clone(&component_1), Arc::clone(&component_2)];
        let result = TensorBlock::new(data, samples.clone(), components, properties.clone());
        assert!(result.is_ok());

        let data = TestArray::new(vec![3, 4, 2]);
        let components = vec![Arc::clone(&component_1), Arc::clone(&component_2)];
        let result = TensorBlock::new(data, samples.clone(), components, properties.clone());
        assert_eq!(
            result.unwrap_err().to_string(),
            "invalid parameter: data and labels don't match: the array has 3 \
            dimensions, but we have 4 separate labels"
        );

        let data = TestArray::new(vec![3, 4, 4, 2]);
        let components = vec![Arc::clone(&component_1), Arc::clone(&component_2)];
        let result = TensorBlock::new(data, samples.clone(), components, properties.clone());
        assert_eq!(
            result.unwrap_err().to_string(),
            "invalid parameter: data and labels don't match: the array shape \
            along axis 2 is 4 but we have 3 entries for the corresponding component"
        );

        let data = TestArray::new(vec![3, 4, 4, 2]);
        let components = vec![Arc::clone(&component_1), Arc::clone(&component_1)];
        let result = TensorBlock::new(data, samples.clone(), components, properties.clone());
        assert_eq!(
            result.unwrap_err().to_string(),
            "invalid parameter: data and labels don't match: some of the \
            component names appear more than once in component labels"
        );

        let data = TestArray::new(vec![3, 1, 2]);
        let mut components = LabelsBuilder::new(vec!["component_1", "component_2"]);
        components.add(&[LabelValue::from(0), LabelValue::from(1)]).unwrap();

        let result = TensorBlock::new(data, samples, vec![Arc::new(components.finish())], properties);
        assert_eq!(
            result.unwrap_err().to_string(),
            "invalid parameter: component labels must have a single dimension, \
            got 2: [component_1, component_2] for component 0"
        );
    }

    mod gradients {
        use super::*;

        #[test]
        fn values_without_components() {
            let samples = example_labels("samples", 4);
            let properties = example_labels("properties", 7);
            let data = TestArray::new(vec![4, 7]);
            let mut block = TensorBlock::new(data, samples, vec![], properties).unwrap();
            assert!(block.gradients().is_empty());

            let gradient = TestArray::new(vec![3, 7]);
            let mut gradient_samples = LabelsBuilder::new(vec!["sample", "foo"]);
            gradient_samples.add(&[0, 0]).unwrap();
            gradient_samples.add(&[1, 1]).unwrap();
            gradient_samples.add(&[3, -2]).unwrap();
            let gradient_samples = Arc::new(gradient_samples.finish());
            block.add_gradient("foo", gradient, gradient_samples, vec![]).unwrap();

            let gradient = TestArray::new(vec![3, 5, 7]);
            let gradient_samples = example_labels("sample", 3);
            let component = example_labels("component", 5);
            block.add_gradient("component", gradient, gradient_samples, vec![component]).unwrap();

            let mut gradients_list = block.gradients().keys().collect::<Vec<_>>();
            gradients_list.sort_unstable();
            assert_eq!(gradients_list, ["component", "foo"]);

            let basic_block = block.gradients().get("foo").unwrap();
            assert_eq!(basic_block.samples.names(), ["sample", "foo"]);
            assert!(basic_block.components.is_empty());
            assert_eq!(basic_block.properties.names(), ["properties"]);

            let basic_block = block.gradients().get("component").unwrap();
            assert_eq!(basic_block.samples.names(), ["sample"]);
            assert_eq!(basic_block.components.len(), 1);
            assert_eq!(basic_block.components[0].names(), ["component"]);
            assert_eq!(basic_block.properties.names(), ["properties"]);

            assert!(block.gradients().get("baz").is_none());
        }

        #[test]
        fn values_with_components() {
            let samples = example_labels("samples", 4);
            let component = example_labels("component", 5);
            let properties = example_labels("properties", 7);
            let data = TestArray::new(vec![4, 5, 7]);
            let mut block = TensorBlock::new(data, samples, vec![component.clone()], properties).unwrap();

            let gradient = TestArray::new(vec![3, 5, 7]);
            let gradient_samples = example_labels("sample", 3);
            let result = block.add_gradient("basic", gradient, gradient_samples.clone(), vec![component.clone()]);
            assert!(result.is_ok());

            let gradient = TestArray::new(vec![3, 3, 5, 7]);
            let component_2 = example_labels("component_2", 3);
            let components = vec![component_2.clone(), component.clone()];
            let result = block.add_gradient("components", gradient, gradient_samples.clone(), components);
            assert!(result.is_ok());

            let gradient = TestArray::new(vec![3, 3, 5, 7]);
            let components = vec![component, component_2];
            let result = block.add_gradient("wrong", gradient, gradient_samples, components);
            assert_eq!(
                result.unwrap_err().to_string(),
                "invalid parameter: gradients and values components mismatch \
                for values component 0 (the corresponding names are [component])"
            );
        }
    }
}
