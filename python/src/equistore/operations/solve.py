import numpy as np

from ..block import TensorBlock
from ..tensor import TensorMap
from . import _dispatch
from ._utils import _check_maps, _check_same_gradients


def solve(X: TensorMap, Y: TensorMap) -> TensorMap:
    """Solve a linear system among two :py:class:`TensorMap`.

    Solve the linear equation set
    ``Y = X * w`` for the unknown ``w``.
    Where ``Y``, ``X`` and ``w`` are all :py:class:`TensorMap`.
    ``Y`` and ``X`` must have the same ``keys`` and
    all their :py:class:`TensorBlock` must be 2D-square array.

    :param X: a :py:class:`TensorMap` containing the "coefficient" matrices.
    :param Y: a :py:class:`TensorMap` containing the "dependent variable" values.

    :return: a :py:class:`TensorMap` with the same keys of ``Y`` and ``X``,
            and where each :py:class:`TensorBlock` has: the ``sample``
            equal to the ``properties`` of ``Y``;
            and the ``properties`` equal to the ``properties`` of ``X``.
    """
    _check_maps(X, Y, "solve")

    for _, X_block in X:
        shape = X_block.values.shape
        if len(shape) != 2 or (not (shape[0] == shape[1])):
            raise ValueError(
                "the values in each block of X should be a square 2D array"
            )

    blocks = []
    for key, X_block in X:
        Y_block = Y.block(key)
        blocks.append(_solve_block(X_block, Y_block))

    return TensorMap(X.keys, blocks)


def _solve_block(X: TensorBlock, Y: TensorBlock) -> TensorBlock:
    """
    Solve a linear system among two :py:class:`TensorBlock`.
    Solve the linear equation set X * w = Y for the unknown w.
    Where X , w, Y are all :py:class:`TensorBlock`
    """
    # TODO handle properties and samples not in the same order?

    if not np.all(X.samples == Y.samples):
        raise ValueError(
            "X and Y blocks in `solve` should have the same samples in the same order"
        )

    if len(X.components) > 0:
        if len(X.components) != len(Y.components):
            raise ValueError(
                "X and Y blocks in `solve` should have the same components \
                in the same order"
            )

        for X_component, Y_component in zip(X.components, Y.components):
            if not np.all(X_component == Y_component):
                raise ValueError(
                    "X and Y blocks in `solve` should have the same components \
                    in the same order"
                )

    # reshape components together with the samples
    X_n_properties = X.values.shape[-1]
    X_values = X.values.reshape(-1, X_n_properties)

    Y_n_properties = Y.values.shape[-1]
    Y_values = Y.values.reshape(-1, Y_n_properties)

    _check_same_gradients(X, Y, props=None, fname="solve")

    for parameter, X_gradient in X.gradients():
        X_gradient_data = X_gradient.data.reshape(-1, X_n_properties)
        X_values = _dispatch.vstack((X_values, X_gradient_data))

        Y_gradient = Y.gradient(parameter)
        Y_gradient_data = Y_gradient.data.reshape(-1, Y_n_properties)
        Y_values = _dispatch.vstack((Y_values, Y_gradient_data))

    weights = _dispatch.solve(X_values, Y_values)

    return TensorBlock(
        values=weights.T,
        samples=Y.properties,
        components=[],
        properties=X.properties,
    )
