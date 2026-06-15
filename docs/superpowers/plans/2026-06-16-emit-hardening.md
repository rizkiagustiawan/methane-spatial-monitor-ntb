# EMIT Production Hardening Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix production issues in EMIT fallback integration

**Architecture:** Address 4 issues: log spam fix, EMIT pagination, EMIT_BBOX env var, URL validation

**Tech Stack:** Rust, serde, std::env

---

## File Structure

| File | Change |
|------|--------|
| `src/main.rs` | Fix log spam, add pagination |
| `src/config.rs` | Add EMIT_BBOX env var, URL validation |
| `.env.example` | Add EMIT_BBOX |

---

### Task 1: Fix Log Spam in Fallback Logic

**Files:**
- Modify: `src/main.rs:1048-1051`

The current fallback logic uses cumulative error counter which causes log spam after first error.

- [ ] **Step 1: Add per-cycle error tracking**

Find `carbon_mapper_tracker_task` and replace the error check:

```rust
// Before the while loop (around line 979), add:
let mut cycle_errors = 0u64;

// In the error arm (around line 1029), change to:
_ => {
    state.metrics.carbon_mapper_errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    cycle_errors += 1;
}

// After the while loop (around line 1048), replace:
if cycle_errors > 0 {
    info!("Carbon Mapper had {} errors this cycle, EMIT fallback active", cycle_errors);
    cycle_errors = 0;
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "fix: use per-cycle error tracking to prevent log spam"
```

---

### Task 2: Add EMIT Pagination

**Files:**
- Modify: `src/main.rs:1073-1078`

Currently EMIT fetches only first 100 results with no pagination.

- [ ] **Step 1: Add pagination loop**

Replace the EMIT fetch logic:

```rust
let mut next_url = Some(search_url.clone());

while let Some(url) = next_url.take() {
    let payload = json!({
        "collections": [collection],
        "bbox": bbox,
        "datetime": format!("2022-08-01T00:00:00Z/{}", Utc::now().format("%Y-%m-%dT%H:%M:%SZ")),
        "limit": 100
    });

    match state.http_client.post(&url).json(&payload).send().await {
        Ok(res) if res.status().is_success() => {
            if let Ok(stac) = res.json::<EmitStacResponse>().await {
                *state.last_emit_fetch.write().unwrap() = Some(Utc::now());

                for feature in stac.features {
                    // ... existing feature processing ...
                }

                // Check for next page
                next_url = stac.links.iter()
                    .find(|l| l.rel == "next")
                    .map(|l| l.href.clone());
            }
        }
        _ => {
            state.metrics.emit_errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            warn!("EMIT fetch failed for: {}", url);
        }
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: add EMIT pagination for complete data fetch"
```

---

### Task 3: Add EMIT_BBOX Environment Variable

**Files:**
- Modify: `src/config.rs:125`
- Modify: `.env.example`

Currently bbox is hardcoded. Make it configurable.

- [ ] **Step 1: Add EMIT_BBOX parsing**

In `src/config.rs`, replace hardcoded bbox:

```rust
bbox: std::env::var("EMIT_BBOX")
    .unwrap_or_else(|_| "115.40,-9.15,119.45,-8.00".into())
    .split(',')
    .map(|s| s.trim().parse().unwrap_or(0.0))
    .collect(),
```

- [ ] **Step 2: Add to .env.example**

Append to `.env.example`:

```env
# Bounding box for EMIT search (min_lon,min_lat,max_lon,max_lat)
EMIT_BBOX=115.40,-9.15,119.45,-8.00
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check`
Expected: Compiles

- [ ] **Step 4: Commit**

```bash
git add src/config.rs .env.example
git commit -m "feat: make EMIT_BBOX configurable via environment variable"
```

---

### Task 4: Add URL Validation for EMIT_STAC_URL

**Files:**
- Modify: `src/config.rs:170-184`

Add validation for EMIT URL format.

- [ ] **Step 1: Add URL validation**

In `validate()` method, add:

```rust
if self.emit.enabled {
    if self.emit.base_url.is_empty() {
        return Err(AppError::Config("EMIT_STAC_URL cannot be empty when EMIT is enabled".into()));
    }
    if !self.emit.base_url.starts_with("http://") && !self.emit.base_url.starts_with("https://") {
        return Err(AppError::Config("EMIT_STAC_URL must start with http:// or https://".into()));
    }
    if self.emit.bbox.len() != 4 {
        return Err(AppError::Config("EMIT_BBOX must have exactly 4 values (min_lon,min_lat,max_lon,max_lat)".into()));
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add src/config.rs
git commit -m "feat: add validation for EMIT URL and bbox configuration"
```

---

### Task 5: Verify All Changes

- [ ] **Step 1: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 2: Run release build**

Run: `cargo build --release`
Expected: Builds successfully

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat: complete EMIT production hardening"
```

---

## Environment Variables Added

| Variable | Default | Description |
|----------|---------|-------------|
| `EMIT_BBOX` | `115.40,-9.15,119.45,-8.00` | Bounding box for EMIT search |

## Fixes Summary

| Issue | Fix |
|-------|-----|
| Log spam | Per-cycle error tracking |
| No pagination | Added `next` link following |
| Hardcoded bbox | `EMIT_BBOX` env var |
| No URL validation | Check format on startup |
