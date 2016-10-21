/**********************************************************************
 * Copyright (c) 2014, 2015 Gregory Maxwell                          *
 * Distributed under the MIT software license, see the accompanying   *
 * file COPYING or http://www.opensource.org/licenses/mit-license.php.*
 **********************************************************************/

#ifndef _SECP256K1_PEDERSEN_H_
#define _SECP256K1_PEDERSEN_H_

#include "group.h"
#include "scalar.h"

#include <stdint.h>

typedef struct {
    secp256k1_ge_storage (*prec)[16][16]; /* prec[j][i] = 16^j * i * G + U_i */
} secp256k1_pedersen_context;

static void secp256k1_pedersen_context_init(secp256k1_pedersen_context* ctx);
static void secp256k1_pedersen_context_build(secp256k1_pedersen_context* ctx, const secp256k1_callback* cb);
static void secp256k1_pedersen_context_clone(secp256k1_pedersen_context *dst,
                                               const secp256k1_pedersen_context* src, const secp256k1_callback* cb);
static void secp256k1_pedersen_context_clear(secp256k1_pedersen_context* ctx);

static int secp256k1_pedersen_context_is_built(const secp256k1_pedersen_context* ctx);

/** Multiply a small number with the generator: r = gn*G2 */
static void secp256k1_pedersen_ecmult_small(const secp256k1_pedersen_context *ctx, secp256k1_gej *r, uint64_t gn);

/* sec * G + value * G2. */
static void secp256k1_pedersen_ecmult(const secp256k1_ecmult_gen_context *ecmult_gen_ctx,
 const secp256k1_pedersen_context *pedersen_ctx, secp256k1_gej *rj, const secp256k1_scalar *sec, uint64_t value);

#endif
