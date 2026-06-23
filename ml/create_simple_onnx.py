import onnx
from onnx import helper, TensorProto
import numpy as np

# Create a very simple ONNX model manually that takes [1, 2] input and outputs [1, 1]
# This mimics a basic linear layer: output = W * x + B, then sigmoid
# We do this to ensure 100% compatibility with tract-onnx Core without heavy ML libraries

# 1. Inputs and Outputs
# Input shape: [1, 2]
X = helper.make_tensor_value_info('float_input', TensorProto.FLOAT, [1, 2])
# Output shape: [1, 1]
Y = helper.make_tensor_value_info('confidence_output', TensorProto.FLOAT, [1, 1])

# 2. Initializers (Weights and Biases)
# W shape [2, 1] -> [0.1, 0.5]
W_val = np.array([[0.1], [0.5]], dtype=np.float32)
W = helper.make_tensor('W', TensorProto.FLOAT, [2, 1], W_val.flatten().tolist())

# B shape [1] -> [0.2]
B_val = np.array([0.2], dtype=np.float32)
B = helper.make_tensor('B', TensorProto.FLOAT, [1], B_val.tolist())

# 3. Nodes
# MatMul: X * W
node_matmul = helper.make_node(
    'MatMul',
    inputs=['float_input', 'W'],
    outputs=['matmul_out'],
    name='MatMul_1'
)

# Add: matmul_out + B
node_add = helper.make_node(
    'Add',
    inputs=['matmul_out', 'B'],
    outputs=['add_out'],
    name='Add_1'
)

# Sigmoid: 1 / (1 + exp(-add_out)) to bound between 0 and 1
node_sigmoid = helper.make_node(
    'Sigmoid',
    inputs=['add_out'],
    outputs=['confidence_output'],
    name='Sigmoid_1'
)

# 4. Graph and Model
graph_def = helper.make_graph(
    [node_matmul, node_add, node_sigmoid],
    'SimpleFusionModel',
    [X],
    [Y],
    [W, B]
)

# Create model (opset 14 is safe for tract)
opset = onnx.OperatorSetIdProto()
opset.version = 14
model_def = helper.make_model(graph_def, producer_name='geoesg-manual', opset_imports=[opset])
onnx.checker.check_model(model_def)

# 5. Save Model
onnx.save(model_def, 'fusion_model.onnx')
print("Successfully generated valid pure-ONNX Core model (fusion_model.onnx)")
