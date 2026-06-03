#include "solo_client.h"
void solo_client_run(const MinerConfig *cfg, SharedState *state) { (void)cfg; (void)state; }
int  build_header(const JobDesc *job, uint64_t nonce, uint8_t out[HEADER_MAX]) { (void)job; (void)nonce; (void)out; return 0; }
