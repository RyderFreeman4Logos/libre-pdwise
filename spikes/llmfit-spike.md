# Spike: llmfit Integration Feasibility

**Date**: 2026-04-04
**Branch**: feat/implement-tiny
**Status**: Complete

## Summary

`llmfit` is installed on this machine, but it is not runnable in the current environment.
The binary at `/usr/local/bin/llmfit` requires `GLIBC_2.39`, while the host only provides `glibc 2.36`.

That means the exact output of `llmfit system --json` could not be captured locally.
However, the upstream repository exposes enough implementation detail to evaluate the integration:

- `llmfit` already detects the fields libre-pdwise needs: RAM, available RAM, CPU core count, GPU presence, GPU VRAM, backend, and per-GPU details.
- The current upstream code shows a richer internal/API schema than the older embedded CLI help text.
- A `sysinfo`-based fallback is practical for RAM and CPU, but not sufficient on its own for VRAM or backend detection.

## Local Availability Check

### Installation

```bash
$ which llmfit
/usr/local/bin/llmfit

$ mise ls | grep llmfit
cargo:llmfit                                0.8.8 (system)      /usr/local/etc/mise/config.toml  latest
```

### Runtime Attempt

```bash
$ llmfit system --json
llmfit: /lib/x86_64-linux-gnu/libc.so.6: version `GLIBC_2.39' not found (required by llmfit)
```

### Runtime Compatibility Notes

```bash
$ getconf GNU_LIBC_VERSION
glibc 2.36

$ file /usr/local/bin/llmfit
/usr/local/bin/llmfit: ELF 64-bit LSB pie executable, x86-64, dynamically linked, ...
```

## Observed Schema Drift in llmfit

Two different JSON shapes are visible from upstream artifacts:

1. The embedded CLI help string still advertises an older compact shape:

```text
JSON output fields: { system: { cpu, ram_gb, gpu_name, gpu_vram_gb, gpu_backend, unified_memory, os } }
```

2. The current upstream implementation and REST API expose a richer shape with:

- `total_ram_gb`
- `available_ram_gb`
- `cpu_name`
- `cpu_cores` or `total_cpu_cores`
- `has_gpu`
- `gpu_vram_gb`
- `total_gpu_vram_gb`
- `gpu_name`
- `gpu_count`
- `unified_memory`
- `backend`
- `gpus[]`
- `cluster_mode`
- `cluster_node_count`

Integration implication:
do not hardcode a single schema without aliases.
The parser should accept both the compact legacy field names and the richer current field names.

## Complete Output Example

Because the binary could not execute locally, the example below is a reconstructed payload based on:

- the current `SystemSpecs` struct in upstream source
- the upstream JSON display code
- the current host's observed CPU and RAM values
- conservative GPU fields where local execution could not confirm the exact value

Current host facts used for the reconstruction:

```bash
$ uname -srm
Linux 6.1.0-44-amd64 x86_64

$ lscpu | sed -n '1,12p'
Architecture:                            x86_64
CPU(s):                                  20
Model name:                              12th Gen Intel(R) Core(TM) i9-12900H

$ python3 -c "..."
MemTotalGiB=31.05
MemAvailableGiB=14.04

$ lspci | rg -i 'vga|3d|display'
00:02.0 VGA compatible controller: Intel Corporation Alder Lake-P Integrated Graphics Controller (rev 0c)
01:00.0 VGA compatible controller: NVIDIA Corporation GA104 [Geforce RTX 3070 Ti Laptop GPU] (rev a1)
```

Reconstructed `llmfit system --json` example:

```json
{
  "system": {
    "total_ram_gb": 31.05,
    "available_ram_gb": 14.04,
    "cpu_cores": 20,
    "cpu_name": "12th Gen Intel(R) Core(TM) i9-12900H",
    "has_gpu": true,
    "gpu_vram_gb": null,
    "total_gpu_vram_gb": null,
    "gpu_name": "GeForce RTX 3070 Ti Laptop GPU",
    "gpu_count": 1,
    "unified_memory": false,
    "backend": "CUDA",
    "gpus": [
      {
        "name": "GeForce RTX 3070 Ti Laptop GPU",
        "vram_gb": null,
        "backend": "CUDA",
        "count": 1,
        "unified_memory": false
      }
    ],
    "cluster_mode": false,
    "cluster_node_count": 0
  }
}
```

Why `gpu_vram_gb` is `null` in this example:

- local `llmfit` execution was impossible
- `nvidia-smi` is present but the NVIDIA driver is not healthy in this environment
- upstream source can estimate VRAM from GPU name when direct queries fail, but that heuristic result could not be verified locally

If libre-pdwise wants a parser that survives both old and new llmfit builds, also accept this legacy-style variant:

```json
{
  "system": {
    "cpu": "12th Gen Intel(R) Core(TM) i9-12900H",
    "ram_gb": 31.05,
    "gpu_name": "GeForce RTX 3070 Ti Laptop GPU",
    "gpu_vram_gb": null,
    "gpu_backend": "CUDA",
    "unified_memory": false,
    "os": "linux"
  }
}
```

## Field Mapping to libre-pdwise Device Capability Detection

For libre-pdwise, the important outputs are RAM, VRAM, CPU cores, and acceleration backend.
`llmfit` reports memory in GB, so the integration should normalize everything to MiB for internal decisions.

| libre-pdwise need | Preferred llmfit field(s) | Fallback alias | Normalization |
|-------------------|---------------------------|----------------|---------------|
| Total RAM (MB) | `system.total_ram_gb` | `system.ram_gb` | `round(gb * 1024.0)` |
| Available RAM (MB) | `system.available_ram_gb` | none | `round(gb * 1024.0)` |
| Primary GPU VRAM (MB) | `system.gpu_vram_gb` | none | `round(gb * 1024.0)` |
| Total usable VRAM across same-model GPUs (MB) | `system.total_gpu_vram_gb` | none | `round(gb * 1024.0)` |
| CPU cores | `system.cpu_cores` or `system.total_cpu_cores` | none | integer as-is |
| Acceleration backend | `system.backend` | `system.gpu_backend` | string as-is |
| Unified memory flag | `system.unified_memory` | none | boolean as-is |
| Primary GPU name | `system.gpu_name` | none | string as-is |
| All GPU candidates | `system.gpus[]` | none | use per GPU for advanced ranking |

### Recommended Internal Mapping

```rust
struct DeviceCapabilities {
    total_ram_mib: u64,
    available_ram_mib: Option<u64>,
    primary_vram_mib: Option<u64>,
    total_vram_mib: Option<u64>,
    cpu_cores: usize,
    backend: AccelerationBackend,
    unified_memory: bool,
}
```

Suggested backend mapping:

| llmfit backend | libre-pdwise meaning |
|----------------|----------------------|
| `CUDA` | NVIDIA acceleration available |
| `ROCm` | AMD ROCm acceleration available |
| `Vulkan` | generic GPU path, especially AMD/Android fallback |
| `SYCL` | Intel GPU path |
| `Metal` | Apple Silicon GPU path |
| `CPU (ARM)` | CPU-only ARM path |
| `CPU (x86)` | CPU-only x86 path |
| `NPU (Ascend)` | dedicated NPU path |

## sysinfo Fallback Evaluation

### What sysinfo can cover well

`sysinfo` is a good fallback for:

- total RAM
- available RAM
- logical CPU count
- CPU brand/name

Those are the same categories llmfit already uses internally before it adds GPU-specific probing.

### What sysinfo cannot cover by itself

`sysinfo` alone is not enough for:

- VRAM detection
- GPU model detection
- acceleration backend detection
- multi-GPU grouping
- unified memory detection

For those fields, llmfit uses extra platform-specific probes such as:

- `nvidia-smi`
- `rocm-smi`
- Linux `/sys/class/drm`
- `lspci`
- macOS `system_profiler`
- Windows WMI
- Vulkan fallback on Android/Termux-style environments

### Practical fallback design for libre-pdwise

Recommended fallback order:

1. Try `llmfit system --json`.
2. If `llmfit` is missing or not executable, use internal detection:
   - `sysinfo` for RAM and CPU
   - lightweight OS-specific GPU probes only when needed
3. If GPU/backend is still unknown, degrade to a conservative CPU-only recommendation

### Why this fallback is acceptable

For local ASR model recommendation, CPU/RAM already give enough signal to pick a safe baseline:

- tiny / base CPU-only model
- larger local model only when backend and VRAM are confidently known

This matches the rule that optimization failures should degrade silently to a safe non-optimized path.

## Integration Recommendation

### Recommendation

Proceed with llmfit integration, but do not make libre-pdwise hard-dependent on llmfit being present or runnable.

### Proposed strategy

1. Add an adapter that shells out to `llmfit system --json`.
2. Parse the JSON with a tolerant schema that accepts both legacy and current field names.
3. Normalize memory units to MiB immediately after parsing.
4. If `llmfit` is missing, exits non-zero, or returns invalid JSON, fall back to internal detection using `sysinfo`.
5. If backend or VRAM remains unknown after fallback, choose a conservative CPU-safe ASR recommendation.

### Important parser behavior

- Prefer `available_ram_gb` over `total_ram_gb` for runtime fit checks.
- Prefer `gpu_vram_gb` for single-device local inference.
- Keep `total_gpu_vram_gb` available for future multi-GPU logic, but do not make ASR recommendations depend on it yet.
- Treat `backend` as authoritative when present.
- Treat missing VRAM as `unknown`, not `0`.

### Minimal acceptance bar for integration

The adapter is good enough when it can always produce one of these outcomes:

- exact device capabilities from llmfit
- reduced capabilities from sysinfo fallback
- conservative CPU-only recommendation

No user-facing path should fail only because llmfit is unavailable.

## Sources

- llmfit README: https://github.com/AlexsJones/llmfit/blob/main/README.md
- llmfit API docs: https://github.com/AlexsJones/llmfit/blob/main/API.md
- llmfit JSON display code: https://github.com/AlexsJones/llmfit/blob/main/src/display.rs
- llmfit hardware detection: https://github.com/AlexsJones/llmfit/blob/main/llmfit-core/src/hardware.rs
- sysinfo crate docs: https://docs.rs/sysinfo/latest/sysinfo/

## Conclusion

`llmfit` is a good fit as the primary hardware detection provider for libre-pdwise.
The main risk is not missing data quality, but operational availability:
binary compatibility, tool installation, and schema drift across versions.

The correct integration shape is:

- `llmfit` first
- tolerant JSON parsing
- `sysinfo` fallback
- conservative CPU-only degradation
