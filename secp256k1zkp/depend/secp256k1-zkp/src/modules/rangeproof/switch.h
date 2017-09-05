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

/* sec * G3. */
static void secp256k1_switch_ecmult(const secp256k1_pedersen_context *switch_ctx, 
                                    secp256k1_gej *rj, 
                                    const secp256k1_scalar *sec);
#endif
