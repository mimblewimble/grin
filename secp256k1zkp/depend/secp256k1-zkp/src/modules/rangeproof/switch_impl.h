/***********************************************************************
 * Copyright (c) 2017 Gregory Maxwell                                  *
 * Distributed under the MIT software license, see the accompanying    *
 * file COPYING or http://www.opensource.org/licenses/mit-license.php. *
 ***********************************************************************/

#ifndef _SECP256K1_SWITCH_IMPL_H_
#define _SECP256K1_SWITCH_IMPL_H_

#include "switch.h"

/** Alternative-alternative generator for secp256k1.
 *  This is the sha256 of the sha256 of 'g' after DER encoding (without compression),
 *  which happens to be a point on the curve.
 *  sage: gen_h =  hashlib.sha256('0479be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8'.decode('hex'))
 *  sage: gen_j_input = gen_h.hexdigest()
 *  sage: gen_j =  hashlib.sha256(gen_j_input.decode('hex'))
 *  sage: G3 = EllipticCurve ([F (0), F (7)]).lift_x(int(gen_j.hexdigest(),16))
 *  sage: '%x %x'%G3.xy()
 */

static const secp256k1_ge secp256k1_ge_const_g3 = SECP256K1_GE_CONST(
    0xb860f567UL, 0x95fc03f3UL, 0xc2168538UL, 0x3d1b5a2fUL,
    0x2954f49bUL, 0x7e398b8dUL, 0x2a019393UL, 0x3621155fUL,
    0x5bc0f62cUL, 0xd35570acUL, 0xbdc0bd8bUL, 0xfc5a95ceUL,
    0x9a5a5965UL, 0x8b30a903UL, 0xa6fe5d22UL, 0x593a37f5UL
);

/* sec * G3 */
SECP256K1_INLINE static void secp256k1_switch_ecmult(const secp256k1_pedersen_context *switch_ctx, 
  secp256k1_gej *rj, const secp256k1_scalar *sec) {
    secp256k1_pedersen_ecmult_small(switch_ctx, rj, sec);
}

#endif
