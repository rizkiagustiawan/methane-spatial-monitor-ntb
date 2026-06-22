import requests
import numpy as np
from sklearn.neural_network import MLPRegressor
from sklearn.preprocessing import StandardScaler
from sklearn.pipeline import Pipeline
import skl2onnx
from skl2onnx.common.data_types import FloatTensorType

# 1. Fetch synthetic data
print("Fetching synthetic data...")
# Assuming the Rust server is running locally on port 3000
try:
    response = requests.get("http://localhost:3000/api/synthetic/plumes?count=10000")
    data = response.json()["data"]
except Exception as e:
    print(f"Failed to fetch data (is the server running?): {e}")
    print("Using dummy data for export testing.")
    data = [{"emission_rate_kg_hr": 1000.0, "wind_speed_ms": 5.0, "is_detectable_by_s5p": True} for _ in range(100)]

# 2. Prepare dataset
# Inputs: emission_rate_kg_hr, wind_speed_ms
# Output: confidence_score (proxy: 0.9 if detectable and high emission, scaling down)
X = []
y = []

for row in data:
    emission = row["emission_rate_kg_hr"]
    wind = row["wind_speed_ms"]
    
    # Heuristic target for training:
    # High emission (>1000) = high confidence. We want the NN to learn a smooth curve.
    base_confidence = 0.5
    if emission > 1000.0:
        base_confidence += 0.3
    if emission > 500.0:
        base_confidence += 0.1
    # Add a little noise so the NN actually has to learn a smooth mapping
    target = min(1.0, max(0.0, base_confidence + np.random.normal(0, 0.05)))
    
    X.append([emission, wind])
    y.append(target)

X = np.array(X, dtype=np.float32)
y = np.array(y, dtype=np.float32)

# 3. Train Model
print("Training MLP Regressor...")
# Use a pipeline to ensure inputs are scaled (very important for ONNX inference later)
pipeline = Pipeline([
    ('scaler', StandardScaler()),
    ('mlp', MLPRegressor(hidden_layer_sizes=(16, 8), activation='relu', max_iter=500, random_state=42))
])

pipeline.fit(X, y)
print(f"Training R^2 score: {pipeline.score(X, y):.4f}")

# 4. Export to ONNX
print("Exporting to ONNX...")
initial_type = [('float_input', FloatTensorType([None, 2]))]
onnx_model = skl2onnx.convert_sklearn(pipeline, initial_types=initial_type)

with open("fusion_model.onnx", "wb") as f:
    f.write(onnx_model.SerializeToString())

print("Saved fusion_model.onnx")
