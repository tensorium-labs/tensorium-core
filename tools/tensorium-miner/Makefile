# tools/tensorium-miner/Makefile
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

NVCCFLAGS := -arch=$(ARCH) -O3 -Xcompiler "-O3 -pthread" \
             -Xptxas -O3,--warn-on-spills
CXXFLAGS  := -O3 -Wall -std=c++11 -pthread
LDLIBS    += -lssl -lcrypto

SRCS_CU  := gpu_worker.cu tensorhash_kernel.cu
SRCS_CPP := main.cpp solo_client.cpp stratum_client.cpp modes.cpp host_tensorhash.cpp

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

gpu_worker.o: gpu_worker.cu gpu_worker.h common.h solo_client.h host_tensorhash.h tensorhash_params.h
	$(NVCC) $(NVCCFLAGS) -c -o $@ $<

tensorhash_kernel.o: tensorhash_kernel.cu tensorhash.cuh blake2b.cuh tensorhash_params.h host_tensorhash.h
	$(NVCC) $(NVCCFLAGS) -c -o $@ $<

# main.cpp and modes.cpp use cuda_runtime.h — compile via nvcc -x c++
main.o: main.cpp common.h solo_client.h stratum_client.h gpu_worker.h nvml_monitor.h modes.h
	$(NVCC) $(NVCCFLAGS) -x c++ -c -o $@ $<

modes.o: modes.cpp modes.h common.h host_tensorhash.h tensorhash_params.h
	$(NVCC) $(NVCCFLAGS) -x c++ -c -o $@ $<

host_tensorhash.o: host_tensorhash.cpp host_tensorhash.h blake2b.cuh tensorhash_params.h
	$(CXX) $(CXXFLAGS) -c -o $@ $<

solo_client.o: solo_client.cpp solo_client.h common.h
	$(CXX) $(CXXFLAGS) -c -o $@ $<

stratum_client.o: stratum_client.cpp stratum_client.h common.h
	$(CXX) $(CXXFLAGS) -c -o $@ $<

nvml_monitor.o: nvml_monitor.cpp nvml_monitor.h common.h
	$(CXX) $(CXXFLAGS) -c -o $@ $<

# Host-only KAT harness — no GPU/CUDA needed (g++ only).
test-host: test_host_tensorhash.cpp host_tensorhash.cpp host_tensorhash.h blake2b.cuh tensorhash_params.h
	$(CXX) $(CXXFLAGS) -o test_host_tensorhash test_host_tensorhash.cpp host_tensorhash.cpp
	./test_host_tensorhash

install: $(TARGET)
	sudo cp $(TARGET) /usr/local/bin/tensorium-miner
	@echo "Installed tensorium-miner"

clean:
	rm -f $(TARGET) test_host_tensorhash *.o

info:
	@echo "ARCH=$(ARCH)"
	@nvcc --version 2>/dev/null | head -1
	@nvidia-smi --query-gpu=name,compute_cap --format=csv,noheader 2>/dev/null || echo "no GPU detected"
