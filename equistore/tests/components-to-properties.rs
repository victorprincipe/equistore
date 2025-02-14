use equistore::{TensorBlock, TensorMap, Labels};

use ndarray::ArrayD;

mod utils;
use utils::example_labels;

#[test]
fn one_component() {
    let mut block = TensorBlock::new(
        ArrayD::from_elem(vec![3, 2, 3], 1.0),
        example_labels(vec!["samples"], vec![[0], [1], [2]]),
        &[example_labels(vec!["components"], vec![[0], [1]])],
        example_labels(vec!["properties"], vec![[0], [1], [2]]),
    ).unwrap();


    block.add_gradient(
        "parameter",
        ArrayD::from_elem(vec![2, 2, 3], 11.0),
        example_labels(vec!["sample", "parameter"], vec![[0, 2], [1, 2]]),
        &[example_labels(vec!["components"], vec![[0], [1]])],
    ).unwrap();

    let tensor = TensorMap::new(Labels::single(), vec![block]).unwrap();
    let tensor = tensor.components_to_properties(&["components"]).unwrap();

    let block = tensor.block_by_id(0);
    let values = block.values();
    assert_eq!(values.samples.names(), ["samples"]);
    assert_eq!(values.samples.count(), 3);
    assert_eq!(values.samples[0], [0]);
    assert_eq!(values.samples[1], [1]);
    assert_eq!(values.samples[2], [2]);

    assert_eq!(values.components.len(), 0);

    assert_eq!(values.properties.names(), ["components", "properties"]);
    assert_eq!(values.properties.count(), 6);
    assert_eq!(values.properties[0], [0, 0]);
    assert_eq!(values.properties[1], [0, 1]);
    assert_eq!(values.properties[2], [0, 2]);
    assert_eq!(values.properties[3], [1, 0]);
    assert_eq!(values.properties[4], [1, 1]);
    assert_eq!(values.properties[5], [1, 2]);

    assert_eq!(values.data.as_array(), ArrayD::from_elem(vec![3, 6], 1.0));

    let gradient = block.gradient("parameter").unwrap();
    assert_eq!(gradient.samples.names(), ["sample", "parameter"]);
    assert_eq!(gradient.samples.count(), 2);
    assert_eq!(gradient.samples[0], [0, 2]);
    assert_eq!(gradient.samples[1], [1, 2]);

    assert_eq!(gradient.data.as_array(), ArrayD::from_elem(vec![2, 6], 11.0));
}

#[test]
fn multiple_components() {
    let data = ArrayD::from_shape_vec(vec![2, 2, 3, 2], vec![
        1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0, 5.0, 5.0, 6.0, 6.0,
        -1.0, 1.0, -2.0, 2.0, -3.0, 3.0, -4.0, 4.0, -5.0, 5.0, -6.0, 6.0,
    ]).unwrap();

    let components = [
        example_labels(vec!["component_1"], vec![[0], [1]]),
        example_labels(vec!["component_2"], vec![[0], [1], [2]]),
    ];

    let mut block = TensorBlock::new(
        data,
        example_labels(vec!["samples"], vec![[0], [1]]),
        &components,
        example_labels(vec!["properties"], vec![[0], [1]]),
    ).unwrap();

    block.add_gradient(
        "parameter",
        ArrayD::from_elem(vec![3, 2, 3, 2], 11.0),
        example_labels(vec!["sample", "parameter"], vec![[0, 2], [0, 3], [1, 2]]),
        &components,
    ).unwrap();

    let tensor = TensorMap::new(Labels::single(), vec![block]).unwrap();
    let tensor = tensor.components_to_properties(&["component_1"]).unwrap();

    let block = tensor.block_by_id(0);
    let values = block.values();
    assert_eq!(values.samples.names(), ["samples"]);
    assert_eq!(values.samples.count(), 2);
    assert_eq!(values.samples[0], [0]);
    assert_eq!(values.samples[1], [1]);

    assert_eq!(values.components.len(), 1);
    assert_eq!(values.components[0].names(), ["component_2"]);
    assert_eq!(values.components[0].count(), 3);
    assert_eq!(values.components[0][0], [0]);
    assert_eq!(values.components[0][1], [1]);
    assert_eq!(values.components[0][2], [2]);

    assert_eq!(values.properties.names(), ["component_1", "properties"]);
    assert_eq!(values.properties.count(), 4);
    assert_eq!(values.properties[0], [0, 0]);
    assert_eq!(values.properties[1], [0, 1]);
    assert_eq!(values.properties[2], [1, 0]);
    assert_eq!(values.properties[3], [1, 1]);

    let expected = ArrayD::from_shape_vec(vec![2, 3, 4], vec![
        1.0, 1.0, 4.0, 4.0, 2.0, 2.0, 5.0, 5.0, 3.0, 3.0, 6.0, 6.0,
        -1.0, 1.0, -4.0, 4.0, -2.0, 2.0, -5.0, 5.0, -3.0, 3.0, -6.0, 6.0,
    ]).unwrap();
    assert_eq!(values.data.as_array(), expected);

    let gradient = block.gradient("parameter").unwrap();
    assert_eq!(gradient.samples.names(), ["sample", "parameter"]);
    assert_eq!(gradient.samples.count(), 3);
    assert_eq!(gradient.samples[0], [0, 2]);
    assert_eq!(gradient.samples[1], [0, 3]);
    assert_eq!(gradient.samples[2], [1, 2]);

    assert_eq!(gradient.data.as_array(), ArrayD::from_elem(vec![3, 3, 4], 11.0));
}
