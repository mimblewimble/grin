/**********************************************************************
 * Copyright (c) 2015 Gregory Maxwell                                 *
 * Distributed under the MIT software license, see the accompanying   *
 * file COPYING or http://www.opensource.org/licenses/mit-license.php.*
 **********************************************************************/

#ifndef _SECP256K1_RANGEPROOF_H_
#define _SECP256K1_RANGEPROOF_H_

#include "scalar.h"
#include "group.h"

typedef struct {
    secp256k1_ge_storage (*prec)[1005];
} secp256k1_rangeproof_context;


static void secp256k1_rangeproof_context_init(secp256k1_rangeproof_context* ctx);
static void secp256k1_rangeproof_context_build(secp256k1_rangeproof_context* ctx, const secp256k1_callback* cb);
static void secp256k1_rangeproof_context_clone(secp256k1_rangeproof_context *dst,
                                               const secp256k1_rangeproof_context* src, const secp256k1_callback* cb);
static void secp256k1_rangeproof_context_clear(secp256k1_rangeproof_context* ctx);
static int secp256k1_rangeproof_context_is_built(const secp256k1_rangeproof_context* ctx);

static int secp256k1_rangeproof_verify_impl(const secp256k1_ecmult_context* ecmult_ctx,
 const secp256k1_ecmult_gen_context* ecmult_gen_ctx,
 const secp256k1_pedersen_context* pedersen_ctx, const secp256k1_rangeproof_context* rangeproof_ctx,
 unsigned char *blindout, uint64_t *value_out, unsigned char *message_out, int *outlen, const unsigned char *nonce,
 uint64_t *min_value, uint64_t *max_value, const unsigned char *commit, const unsigned char *proof, int plen);

#endif
