from __future__ import annotations

import os
from pathlib import Path

import jax

if "JAX_COMPILATION_CACHE_DIR" not in os.environ:
    _cache_dir = Path("/tmp/jax_cache_gen2_tpu")
    try:
        _cache_dir.mkdir(parents=True, exist_ok=True)
        jax.config.update("jax_compilation_cache_dir", str(_cache_dir))
        jax.config.update("jax_persistent_cache_min_compile_time_secs", 0.5)
    except Exception:
        pass
