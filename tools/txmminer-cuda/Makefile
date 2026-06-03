# tools/txmminer-cuda/Makefile
NVCC    ?= nvcc
CXX     ?= g++
TARGET  ?= tensorium-miner

ifndef ARCH
  DETECTED := $(shell nvidia-smi --query-gpu=compute_cap --format=csv,noheader 2>/dev/null | head -1 | tr -d .)
  ifneq ($(DETECTED),)
    ARCH := sm_$(DETECTED)
  else
    ARCH := sm_86
    $(warning nvidia-smi not found — defaulting to $(ARCH))
  endif
endif

NVCCFLAGS := -arch=$(ARCH) -O3 --use_fast_math -Xcompiler "-O3 -pthread" \
             -Xptxas -O3,--warn-on-spills
CXXFLAGS  := -O3 -Wall -std=c++11 -pthread

SRCS_CU  := gpu_worker.cu mining_kernel.cu
SRCS_CPP := main.cpp solo_client.cpp stratum_client.cpp

ifdef WITH_NVML
  SRCS_CPP  += nvml_monitor.cpp
  NVCCFLAGS += -DWITH_NVML
  CXXFLAGS  += -DWITH_NVML
  LDLIBS    += -lnvidia-ml
endif

OBJS_CU  := $(SRCS_CU:.cu=.o)
OBJS_CPP := $(SRCS_CPP:.cpp=.o)

all: $(TARGET)

$(TARGET): $(OBJS_CU) $(OBJS_CPP)
	$(NVCC) -arch=$(ARCH) -Xcompiler -pthread -o $@ $^ $(LDLIBS)
	@echo ""
	@echo "Built: $(TARGET) ($(ARCH))"

# Explicit rules for CUDA sources (only gpu_worker.cu and mining_kernel.cu)
gpu_worker.o: gpu_worker.cu sha256d.cuh common.h gpu_worker.h
	$(NVCC) $(NVCCFLAGS) -c -o $@ $<

mining_kernel.o: mining_kernel.cu sha256d.cuh common.h
	$(NVCC) $(NVCCFLAGS) -c -o $@ $<

# Explicit rules for C++ sources (main.cpp, not main.cu)
main.o: main.cpp common.h solo_client.h stratum_client.h gpu_worker.h nvml_monitor.h
	$(CXX) $(CXXFLAGS) -c -o $@ $<

solo_client.o: solo_client.cpp solo_client.h common.h
	$(CXX) $(CXXFLAGS) -c -o $@ $<

stratum_client.o: stratum_client.cpp stratum_client.h common.h
	$(CXX) $(CXXFLAGS) -c -o $@ $<

nvml_monitor.o: nvml_monitor.cpp nvml_monitor.h common.h
	$(CXX) $(CXXFLAGS) -c -o $@ $<

install: $(TARGET)
	sudo cp $(TARGET) /usr/local/bin/tensorium-miner
	sudo ln -sf /usr/local/bin/tensorium-miner /usr/local/bin/txmminer-cuda
	@echo "Installed tensorium-miner; txmminer-cuda -> tensorium-miner"

clean:
	rm -f $(TARGET) txmminer-cuda *.o

info:
	@echo "ARCH=$(ARCH)"
	@nvcc --version 2>/dev/null | head -1
	@nvidia-smi --query-gpu=name,compute_cap --format=csv,noheader 2>/dev/null || echo "no GPU detected"
