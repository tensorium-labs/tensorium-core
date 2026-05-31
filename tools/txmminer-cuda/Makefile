# Tensorium CUDA Miner — Makefile
#
# Requirements:
#   - NVIDIA CUDA Toolkit 11.0+ (nvcc)
#   - GCC or Clang
#   - GPU with CUDA Compute Capability 6.0+ (Pascal and newer)
#
# Build:
#   make                      # auto-detect GPU arch
#   make ARCH=sm_89           # RTX 4000 series
#   make ARCH=sm_86           # RTX 3000 series
#   make ARCH=sm_80           # A100 / RTX 30 series (Ampere)
#   make ARCH=sm_75           # RTX 20 series (Turing)
#   make ARCH=sm_61           # GTX 10 series (Pascal)
#
# Common architectures:
#   sm_61 → GTX 1060/1070/1080
#   sm_75 → RTX 2060/2070/2080
#   sm_80 → A100, RTX 30 series (Ampere)
#   sm_86 → RTX 3060/3070/3080/3090
#   sm_89 → RTX 4060/4070/4080/4090 (Ada Lovelace)
#   sm_90 → H100
#   sm_100 → H200 / Blackwell

NVCC    ?= nvcc
CC      ?= gcc
TARGET  ?= txmminer-cuda

# Auto-detect GPU architecture if not specified
ifndef ARCH
  DETECTED := $(shell nvidia-smi --query-gpu=compute_cap --format=csv,noheader 2>/dev/null | head -1 | tr -d .)
  ifneq ($(DETECTED),)
    ARCH := sm_$(DETECTED)
  else
    ARCH := sm_86
    $(warning "nvidia-smi not found, defaulting to $(ARCH). Set ARCH= if wrong.")
  endif
endif

NVCCFLAGS := -arch=$(ARCH) -O3 --use_fast_math -Xcompiler -O3
CFLAGS    := -O3 -Wall

.PHONY: all clean info

all: $(TARGET)

$(TARGET): main.c mining_kernel.cu sha256d.cuh
	$(NVCC) $(NVCCFLAGS) -o $@ main.c mining_kernel.cu
	@echo ""
	@echo "Built: $(TARGET) for $(ARCH)"
	@echo "Run:   ./$(TARGET) 127.0.0.1:23332 txm1youraddress"

info:
	@echo "ARCH=$(ARCH)"
	@nvcc --version 2>/dev/null || echo "nvcc not found"
	@nvidia-smi --query-gpu=name,compute_cap --format=csv,noheader 2>/dev/null || echo "no GPU detected"

clean:
	rm -f $(TARGET) *.o *.ptx
