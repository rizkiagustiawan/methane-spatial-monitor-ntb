import requests
import numpy as np
import torch
import torch.nn as nn
import torch.optim as optim

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
X = []
y = []

# Manual scaling parameters (simplistic min-max approach for this example)
# Emission typically 0 - 5000, Wind typically 0 - 20
MAX_EMISSION = 5000.0
MAX_WIND = 20.0

for row in data:
    emission = row["emission_rate_kg_hr"]
    wind = row["wind_speed_ms"]
    
    # Scale inputs manually between 0 and 1 so we don't need StandardScaler in ONNX
    emission_scaled = min(emission / MAX_EMISSION, 1.0)
    wind_scaled = min(wind / MAX_WIND, 1.0)
    
    base_confidence = 0.5
    if emission > 1000.0:
        base_confidence += 0.3
    if emission > 500.0:
        base_confidence += 0.1
    
    target = min(1.0, max(0.0, base_confidence + np.random.normal(0, 0.05)))
    
    X.append([emission_scaled, wind_scaled])
    y.append([target])

X_tensor = torch.tensor(X, dtype=torch.float32)
y_tensor = torch.tensor(y, dtype=torch.float32)

# 3. Define and Train PyTorch Model
class FusionMLP(nn.Module):
    def __init__(self):
        super(FusionMLP, self).__init__()
        self.layers = nn.Sequential(
            nn.Linear(2, 16),
            nn.ReLU(),
            nn.Linear(16, 8),
            nn.ReLU(),
            nn.Linear(8, 1),
            nn.Sigmoid() # Bound output between 0 and 1
        )

    def forward(self, x):
        # We enforce the manual scaling inside the Rust backend instead of the ONNX graph
        return self.layers(x)

model = FusionMLP()
criterion = nn.MSELoss()
optimizer = optim.Adam(model.parameters(), lr=0.01)

print("Training PyTorch Model...")
epochs = 500
for epoch in range(epochs):
    optimizer.zero_grad()
    outputs = model(X_tensor)
    loss = criterion(outputs, y_tensor)
    loss.backward()
    optimizer.step()
    
print(f"Final Loss: {loss.item():.4f}")

# 4. Export to ONNX
print("Exporting to ONNX...")
# Create dummy input with exact shape [1, 2] required by Tract
dummy_input = torch.randn(1, 2)
torch.onnx.export(
    model, 
    dummy_input, 
    "fusion_model.onnx", 
    export_params=True,
    opset_version=14, # Tract supports up to 18 safely
    do_constant_folding=True,
    input_names=['float_input'],
    output_names=['confidence_output'],
    dynamic_axes=None # Absolute shape, no dynamic axes
)

print("Saved fusion_model.onnx (Pure PyTorch Core ONNX)")