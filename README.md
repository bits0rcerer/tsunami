# 🌊 Tsuanmi 🌊

A potent GPU accelerated Pixelflut client for modern Linux written in Rust 🦀

## Highlights

- GPU acceleration
- io-uring

## But is it fast?

Yes. It even outperforms [sturmflut](https://github.com/TobleMiner/sturmflut) on a Surface Book 2.
But I yet have to test it on bigger machines.

## Requirements

- modern linux kernel with io-uring (**>6.0**, >5.8 may work as well)
- Vulkan and a GPU
- [krnlc](https://docs.rs/krnl/latest/krnl/kernel/index.html#compiling)

## How to

```bash
# compile spirv kernels
krnlc -p epizentrum

# compile tsunami
cargo build --release --package tsunami

# compile with optimization for your CPU
RUSTFLAGS='-C target-cpu=native' cargo build --release --package tsunami

./target/release/tsunami help
```

## 👋 See you at 37c3
