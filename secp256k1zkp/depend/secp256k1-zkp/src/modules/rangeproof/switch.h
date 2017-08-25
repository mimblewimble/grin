/**********************************************************************
 * Copyright (c) 2014, 2017 Gregory Maxwell                          *
 * Distributed under the MIT software license, see the accompanying   *
 * file COPYING or http://www.opensource.org/licenses/mit-license.php.*
 **********************************************************************/

#ifndef _SECP256K1_SWITCH_H_
#define _SECP256K1_SWITCH_H_

#include "group.h"
#include "scalar.h"

#include <stdint.h>

typedef struct {
    secp256k1_ge_storage (*prec)[16][16]; /* prec[j][i] = 16^j * i * G + U_i */
} secp256k1_switch_context;

static void secp256k1_switch_context_init(secp256k1_switch_context* ctx);
static void secp256k1_switch_context_build(secp256k1_switch_context* ctx, const secp256k1_callback* cb);
static void secp256k1_switch_context_clone(secp256k1_switch_context *dst,
                                               const secp256k1_switch_context* src, const secp256k1_callback* cb);
static void secp256k1_switch_context_clear(secp256k1_switch_context* ctx);

static int secp256k1_switch_context_is_built(const secp256k1_switch_context* ctx);

/* sec * G3. */
static void secp256k1_switch_ecmult(const secp256k1_switch_context *switch_ctx, 
                                    secp256k1_gej *rj, 
                                    const secp256k1_scalar *sec);
#endif
