/***********************************************************************
 * Copyright (c) 2015 Gregory Maxwell                                  *
 * Distributed under the MIT software license, see the accompanying    *
 * file COPYING or http://www.opensource.org/licenses/mit-license.php. *
 ***********************************************************************/

#ifndef _SECP256K1_PEDERSEN_IMPL_H_
#define _SECP256K1_PEDERSEN_IMPL_H_

/** Alternative generator for secp256k1.
 *  This is the sha256 of 'g' after DER encoding (without compression),
 *  which happens to be a point on the curve.
 *  sage: G2 = EllipticCurve ([F (0), F (7)]).lift_x(int(hashlib.sha256('0479be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8'.decode('hex')).hexdigest(),16))
 *  sage: '%x %x'%G2.xy()
 */
static const secp256k1_ge secp256k1_ge_const_g2 = SECP256K1_GE_CONST(
    0x50929b74UL, 0xc1a04954UL, 0xb78b4b60UL, 0x35e97a5eUL,
    0x078a5a0fUL, 0x28ec96d5UL, 0x47bfee9aUL, 0xce803ac0UL,
    0x31d3c686UL, 0x3973926eUL, 0x049e637cUL, 0xb1b5f40aUL,
    0x36dac28aUL, 0xf1766968UL, 0xc30c2313UL, 0xf3a38904UL
);

static void secp256k1_pedersen_context_init(secp256k1_pedersen_context *ctx) {
    ctx->prec = NULL;
}

static void secp256k1_pedersen_context_build(secp256k1_pedersen_context *ctx, const secp256k1_callback *cb) {
    secp256k1_ge prec[256];
    secp256k1_gej gj;
    secp256k1_gej nums_gej;
    int i, j;

    if (ctx->prec != NULL) {
        return;
    }

    ctx->prec = (secp256k1_ge_storage (*)[16][16])checked_malloc(cb, sizeof(*ctx->prec));

    /* get the generator */
    secp256k1_gej_set_ge(&gj, &secp256k1_ge_const_g2);

    /* Construct a group element with no known corresponding scalar (nothing up my sleeve). */
    {
        static const unsigned char nums_b32[33] = "The scalar for this x is unknown";
        secp256k1_fe nums_x;
        secp256k1_ge nums_ge;
        VERIFY_CHECK(secp256k1_fe_set_b32(&nums_x, nums_b32));
        VERIFY_CHECK(secp256k1_ge_set_xo_var(&nums_ge, &nums_x, 0));
        secp256k1_gej_set_ge(&nums_gej, &nums_ge);
        /* Add G to make the bits in x uniformly distributed. */
        secp256k1_gej_add_ge_var(&nums_gej, &nums_gej, &secp256k1_ge_const_g2, NULL);
    }

    /* compute prec. */
    {
        secp256k1_gej precj[256]; /* Jacobian versions of prec. */
        secp256k1_gej gbase;
        secp256k1_gej numsbase;
        gbase = gj; /* 16^j * G */
        numsbase = nums_gej; /* 2^j * nums. */
        for (j = 0; j < 16; j++) {
            /* Set precj[j*16 .. j*16+15] to (numsbase, numsbase + gbase, ..., numsbase + 15*gbase). */
            precj[j*16] = numsbase;
            for (i = 1; i < 16; i++) {
                secp256k1_gej_add_var(&precj[j*16 + i], &precj[j*16 + i - 1], &gbase, NULL);
            }
            /* Multiply gbase by 16. */
            for (i = 0; i < 4; i++) {
                secp256k1_gej_double_var(&gbase, &gbase, NULL);
            }
            /* Multiply numbase by 2. */
            secp256k1_gej_double_var(&numsbase, &numsbase, NULL);
            if (j == 14) {
                /* In the last iteration, numsbase is (1 - 2^j) * nums instead. */
                secp256k1_gej_neg(&numsbase, &numsbase);
                secp256k1_gej_add_var(&numsbase, &numsbase, &nums_gej, NULL);
            }
        }
        secp256k1_ge_set_all_gej_var(256, prec, precj, cb);
    }
    for (j = 0; j < 16; j++) {
        for (i = 0; i < 16; i++) {
            secp256k1_ge_to_storage(&(*ctx->prec)[j][i], &prec[j*16 + i]);
        }
    }
}

static int secp256k1_pedersen_context_is_built(const secp256k1_pedersen_context* ctx) {
    return ctx->prec != NULL;
}

static void secp256k1_pedersen_context_clone(secp256k1_pedersen_context *dst,
                                               const secp256k1_pedersen_context *src, const secp256k1_callback *cb) {
    if (src->prec == NULL) {
        dst->prec = NULL;
    } else {
        dst->prec = (secp256k1_ge_storage (*)[16][16])checked_malloc(cb, sizeof(*dst->prec));
        memcpy(dst->prec, src->prec, sizeof(*dst->prec));
    }
}

static void secp256k1_pedersen_context_clear(secp256k1_pedersen_context *ctx) {
    free(ctx->prec);
    ctx->prec = NULL;
}

/* Version of secp256k1_ecmult_gen using the second generator and working only on numbers in the range [0 .. 2^64). */
static void secp256k1_pedersen_ecmult_small(const secp256k1_pedersen_context *ctx, secp256k1_gej *r, uint64_t gn) {
    secp256k1_ge add;
    secp256k1_ge_storage adds;
    int bits;
    int i, j;
    memset(&adds, 0, sizeof(adds));
    secp256k1_gej_set_infinity(r);
    add.infinity = 0;
    for (j = 0; j < 16; j++) {
        bits = (gn >> (j * 4)) & 15;
        for (i = 0; i < 16; i++) {
            secp256k1_ge_storage_cmov(&adds, &(*ctx->prec)[j][i], i == bits);
        }
        secp256k1_ge_from_storage(&add, &adds);
        secp256k1_gej_add_ge(r, r, &add);
    }
    bits = 0;
    secp256k1_ge_clear(&add);
}

/* sec * G + value * G2. */
SECP256K1_INLINE static void secp256k1_pedersen_ecmult(const secp256k1_ecmult_gen_context *ecmult_gen_ctx,
 const secp256k1_pedersen_context *pedersen_ctx, secp256k1_gej *rj, const secp256k1_scalar *sec, uint64_t value) {
    secp256k1_gej vj;
    secp256k1_ecmult_gen(ecmult_gen_ctx, rj, sec);
    secp256k1_pedersen_ecmult_small(pedersen_ctx, &vj, value);
    /* FIXME: constant time. */
    secp256k1_gej_add_var(rj, rj, &vj, NULL);
    secp256k1_gej_clear(&vj);
}

#endif
