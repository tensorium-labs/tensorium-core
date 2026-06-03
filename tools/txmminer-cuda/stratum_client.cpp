#include "stratum_client.h"
#include <stdio.h>
#include <unistd.h>
void stratum_client_run(const MinerConfig *cfg, SharedState *state) {
    fprintf(stderr, "[pool] stratum client not yet implemented\n");
    while (state->running) { sleep(1); }
}
