/**********************************************************************
 * Copyright (c) 2015 Gregory Maxwell                                 *
 * Distributed under the MIT software license, see the accompanying   *
 * file COPYING or http://www.opensource.org/licenses/mit-license.php.*
 **********************************************************************/

#ifndef _SECP256K1_RANGEPROOF_IMPL_H_
#define _SECP256K1_RANGEPROOF_IMPL_H_

#include "scalar.h"
#include "group.h"
#include "rangeproof.h"
#include "hash_impl.h"

#include "modules/rangeproof/pedersen.h"
#include "modules/rangeproof/borromean.h"

static const int secp256k1_rangeproof_offsets[20] = {
      0,  96, 189, 276, 360, 438, 510, 579, 642,
    699, 753, 801, 843, 882, 915, 942, 966, 984,
    996, 1005,
};

static void secp256k1_rangeproof_context_init(secp256k1_rangeproof_context *ctx) {
    ctx->prec = NULL;
}

static void secp256k1_rangeproof_context_build(secp256k1_rangeproof_context *ctx, const secp256k1_callback* cb) {
    secp256k1_ge *prec;
    secp256k1_gej *precj;
    secp256k1_gej gj;
    secp256k1_gej one;
    int i, pos;

    if (ctx->prec != NULL) {
        return;
    }

    precj = (secp256k1_gej (*))checked_malloc(cb, sizeof(*precj) * 1005);
    if (precj == NULL) {
        return;
    }
    prec = (secp256k1_ge (*))checked_malloc(cb, sizeof(*prec) * 1005);
    if (prec == NULL) {
        free(precj);
        return;
    }

    /* get the generator */
    secp256k1_gej_set_ge(&one, &secp256k1_ge_const_g2);
    secp256k1_gej_neg(&one, &one);

    /* compute prec. */
    pos = 0;
    for (i = 0; i < 19; i++) {
        int pmax;
        pmax = secp256k1_rangeproof_offsets[i + 1];
        gj = one;
        while (pos < pmax) {
            precj[pos] = gj;
            pos++;
            secp256k1_gej_double_var(&precj[pos], &gj, NULL);
            pos++;
            secp256k1_gej_add_var(&precj[pos], &precj[pos - 1], &gj, NULL);
            pos++;
            if (pos < pmax - 1) {
                secp256k1_gej_double_var(&gj, &precj[pos - 2], NULL);
            }
        }
        if (i < 18) {
            secp256k1_gej_double_var(&gj, &one, NULL);
            one = gj;
            secp256k1_gej_double_var(&gj, &gj, NULL);
            secp256k1_gej_double_var(&gj, &gj, NULL);
            secp256k1_gej_add_var(&one, &one, &gj, NULL);
        }
    }
    VERIFY_CHECK(pos == 1005);
    secp256k1_ge_set_all_gej_var(1005, prec, precj, cb);

    free(precj);

    ctx->prec = (secp256k1_ge_storage (*)[1005])checked_malloc(cb, sizeof(*ctx->prec));
    if (ctx->prec == NULL) {
        free(prec);
        return;
    }

    for (i = 0; i < 1005; i++) {
        secp256k1_ge_to_storage(&(*ctx->prec)[i], &prec[i]);
    }
    free(prec);
}


static int secp256k1_rangeproof_context_is_built(const secp256k1_rangeproof_context* ctx) {
    return ctx->prec != NULL;
}

static void secp256k1_rangeproof_context_clone(secp256k1_rangeproof_context *dst,
                                               const secp256k1_rangeproof_context *src, const secp256k1_callback* cb) {
    if (src->prec == NULL) {
        dst->prec = NULL;
    } else {
        dst->prec = (secp256k1_ge_storage (*)[1005])checked_malloc(cb, sizeof(*dst->prec));
        memcpy(dst->prec, src->prec, sizeof(*dst->prec));
    }
}

static void secp256k1_rangeproof_context_clear(secp256k1_rangeproof_context *ctx) {
    free(ctx->prec);
    ctx->prec = NULL;
}

SECP256K1_INLINE static void secp256k1_rangeproof_pub_expand(const secp256k1_rangeproof_context *ctx, secp256k1_gej *pubs,
 int exp, int *rsizes, int rings) {
    secp256k1_ge ge;
    secp256k1_ge_storage *basis;
    int i;
    int j;
    int npub;
    VERIFY_CHECK(exp < 19);
    if (exp < 0) {
        exp = 0;
    }
    basis = &(*ctx->prec)[secp256k1_rangeproof_offsets[exp]];
    npub = 0;
    for (i = 0; i < rings; i++) {
        for (j = 1; j < rsizes[i]; j++) {
            secp256k1_ge_from_storage(&ge, &basis[i * 3 + j - 1]);
            secp256k1_gej_add_ge_var(&pubs[npub + j], &pubs[npub], &ge, NULL);
        }
        npub += rsizes[i];
    }
}

SECP256K1_INLINE static int secp256k1_rangeproof_genrand(secp256k1_scalar *sec, secp256k1_scalar *s, unsigned char *message,
 int *rsizes, int rings, const unsigned char *nonce, const unsigned char *commit, const unsigned char *proof, int len) {
    unsigned char tmp[32];
    unsigned char rngseed[32 + 33 + 10];
    secp256k1_rfc6979_hmac_sha256_t rng;
    secp256k1_scalar acc;
    int overflow;
    int ret;
    int i;
    int j;
    int b;
    int npub;
    VERIFY_CHECK(len <= 10);
    memcpy(rngseed, nonce, 32);
    memcpy(rngseed + 32, commit, 33);
    memcpy(rngseed + 65, proof, len);
    secp256k1_rfc6979_hmac_sha256_initialize(&rng, rngseed, 32 + 33 + len);
    secp256k1_scalar_clear(&acc);
    npub = 0;
    ret = 1;
    for (i = 0; i < rings; i++) {
        if (i < rings - 1) {
            secp256k1_rfc6979_hmac_sha256_generate(&rng, tmp, 32);
            do {
                secp256k1_rfc6979_hmac_sha256_generate(&rng, tmp, 32);
                secp256k1_scalar_set_b32(&sec[i], tmp, &overflow);
            } while (overflow || secp256k1_scalar_is_zero(&sec[i]));
            secp256k1_scalar_add(&acc, &acc, &sec[i]);
        } else {
            secp256k1_scalar_negate(&acc, &acc);
            sec[i] = acc;
        }
        for (j = 0; j < rsizes[i]; j++) {
            secp256k1_rfc6979_hmac_sha256_generate(&rng, tmp, 32);
            if (message) {
                for (b = 0; b < 32; b++) {
                    tmp[b] ^= message[(i * 4 + j) * 32 + b];
                    message[(i * 4 + j) * 32 + b] = tmp[b];
                }
            }
            secp256k1_scalar_set_b32(&s[npub], tmp, &overflow);
            ret &= !(overflow || secp256k1_scalar_is_zero(&s[npub]));
            npub++;
        }
    }
    secp256k1_rfc6979_hmac_sha256_finalize(&rng);
    secp256k1_scalar_clear(&acc);
    memset(tmp, 0, 32);
    return ret;
}

SECP256K1_INLINE static int secp256k1_range_proveparams(uint64_t *v, int *rings, int *rsizes, int *npub, int *secidx, uint64_t *min_value,
 int *mantissa, uint64_t *scale,  int *exp, int *min_bits, uint64_t value) {
    int i;
    *rings = 1;
    rsizes[0] = 1;
    secidx[0] = 0;
    *scale = 1;
    *mantissa = 0;
    *npub = 0;
    if (*min_value == UINT64_MAX) {
        /* If the minimum value is the maximal representable value, then we cannot code a range. */
        *exp = -1;
    }
    if (*exp >= 0) {
        int max_bits;
        uint64_t v2;
        if ((*min_value && value > INT64_MAX) || (value && *min_value >= INT64_MAX)) {
            /* If either value or min_value is >= 2^63-1 then the other must by zero to avoid overflowing the proven range. */
            return 0;
        }
        max_bits = *min_value ? secp256k1_clz64_var(*min_value) : 64;
        if (*min_bits > max_bits) {
            *min_bits = max_bits;
        }
        if (*min_bits > 61 || value > INT64_MAX) {
            /** Ten is not a power of two, so dividing by ten and then representing in base-2 times ten
             *   expands the representable range. The verifier requires the proven range is within 0..2**64.
             *   For very large numbers (all over 2**63) we must change our exponent to compensate.
             *  Rather than handling it precisely, this just disables use of the exponent for big values.
             */
            *exp = 0;
        }
        /* Mask off the least significant digits, as requested. */
        *v = value - *min_value;
        /* If the user has asked for more bits of proof then there is room for in the exponent, reduce the exponent. */
        v2 = *min_bits ? (UINT64_MAX>>(64-*min_bits)) : 0;
        for (i = 0; i < *exp && (v2 <= UINT64_MAX / 10); i++) {
            *v /= 10;
            v2 *= 10;
        }
        *exp = i;
        v2 = *v;
        for (i = 0; i < *exp; i++) {
            v2 *= 10;
            *scale *= 10;
        }
        /* If the masked number isn't precise, compute the public offset. */
        *min_value = value - v2;
        /* How many bits do we need to represent our value? */
        *mantissa = *v ? 64 - secp256k1_clz64_var(*v) : 1;
        if (*min_bits > *mantissa) {
            /* If the user asked for more precision, give it to them. */
            *mantissa = *min_bits;
        }
        /* Digits in radix-4, except for the last digit if our mantissa length is odd. */
        *rings = (*mantissa + 1) >> 1;
        for (i = 0; i < *rings; i++) {
            rsizes[i] = ((i < *rings - 1) | (!(*mantissa&1))) ? 4 : 2;
            *npub += rsizes[i];
            secidx[i] = (*v >> (i*2)) & 3;
        }
        VERIFY_CHECK(*mantissa>0);
        VERIFY_CHECK((*v & ~(UINT64_MAX>>(64-*mantissa))) == 0); /* Did this get all the bits? */
    } else {
        /* A proof for an exact value. */
        *exp = 0;
        *min_value = value;
        *v = 0;
        *npub = 2;
    }
    VERIFY_CHECK(*v * *scale + *min_value == value);
    VERIFY_CHECK(*rings > 0);
    VERIFY_CHECK(*rings <= 32);
    VERIFY_CHECK(*npub <= 128);
    return 1;
}

/* strawman interface, writes proof in proof, a buffer of plen, proves with respect to min_value the range for commit which has the provided blinding factor and value. */
SECP256K1_INLINE static int secp256k1_rangeproof_sign_impl(const secp256k1_ecmult_context* ecmult_ctx,
 const secp256k1_ecmult_gen_context* ecmult_gen_ctx, const secp256k1_pedersen_context* pedersen_ctx,
 const secp256k1_rangeproof_context* rangeproof_ctx, unsigned char *proof, int *plen, uint64_t min_value,
 const unsigned char *commit, const unsigned char *blind, const unsigned char *nonce, int exp, int min_bits, uint64_t value){
    secp256k1_gej pubs[128];     /* Candidate digits for our proof, most inferred. */
    secp256k1_scalar s[128];     /* Signatures in our proof, most forged. */
    secp256k1_scalar sec[32];    /* Blinding factors for the correct digits. */
    secp256k1_scalar k[32];      /* Nonces for our non-forged signatures. */
    secp256k1_scalar stmp;
    secp256k1_sha256_t sha256_m;
    unsigned char prep[4096];
    unsigned char tmp[33];
    unsigned char *signs;          /* Location of sign flags in the proof. */
    uint64_t v;
    uint64_t scale;                /* scale = 10^exp. */
    int mantissa;                  /* Number of bits proven in the blinded value. */
    int rings;                     /* How many digits will our proof cover. */
    int rsizes[32];                /* How many possible values there are for each place. */
    int secidx[32];                /* Which digit is the correct one. */
    int len;                       /* Number of bytes used so far. */
    int i;
    int overflow;
    int npub;
    len = 0;
    if (*plen < 65 || min_value > value || min_bits > 64 || min_bits < 0 || exp < -1 || exp > 18) {
        return 0;
    }
    if (!secp256k1_range_proveparams(&v, &rings, rsizes, &npub, secidx, &min_value, &mantissa, &scale, &exp, &min_bits, value)) {
        return 0;
    }
    proof[len] = (rsizes[0] > 1 ? (64 | exp) : 0) | (min_value ? 32 : 0);
    len++;
    if (rsizes[0] > 1) {
        VERIFY_CHECK(mantissa > 0 && mantissa <= 64);
        proof[len] = mantissa - 1;
        len++;
    }
    if (min_value) {
        for (i = 0; i < 8; i++) {
            proof[len + i] = (min_value >> ((7-i) * 8)) & 255;
        }
        len += 8;
    }
    /* Do we have enough room for the proof? */
    if (*plen - len < 32 * (npub + rings - 1) + 32 + ((rings+6) >> 3)) {
        return 0;
    }
    secp256k1_sha256_initialize(&sha256_m);
    secp256k1_sha256_write(&sha256_m, commit, 33);
    secp256k1_sha256_write(&sha256_m, proof, len);

    memset(prep, 0, 4096);
    /* Note, the data corresponding to the blinding factors must be zero. */
    if (rsizes[rings - 1] > 1) {
        int idx;
        /* Value encoding sidechannel. */
        idx = rsizes[rings - 1] - 1;
        idx -= secidx[rings - 1] == idx;
        idx = ((rings - 1) * 4 + idx) * 32;
        for (i = 0; i < 8; i++) {
            prep[8 + i + idx] = prep[16 + i + idx] = prep[24 + i + idx] = (v >> (56 - i * 8)) & 255;
            prep[i + idx] = 0;
        }
        prep[idx] = 128;
    }
    if (!secp256k1_rangeproof_genrand(sec, s, prep, rsizes, rings, nonce, commit, proof, len)) {
        return 0;
    }
    memset(prep, 0, 4096);
    for (i = 0; i < rings; i++) {
        /* Sign will overwrite the non-forged signature, move that random value into the nonce. */
        k[i] = s[i * 4 + secidx[i]];
        secp256k1_scalar_clear(&s[i * 4 + secidx[i]]);
    }
    /** Genrand returns the last blinding factor as -sum(rest),
     *   adding in the blinding factor for our commitment, results in the blinding factor for
     *   the commitment to the last digit that the verifier can compute for itself by subtracting
     *   all the digits in the proof from the commitment. This lets the prover skip sending the
     *   blinded value for one digit.
     */
    secp256k1_scalar_set_b32(&stmp, blind, &overflow);
    secp256k1_scalar_add(&sec[rings - 1], &sec[rings - 1], &stmp);
    if (overflow || secp256k1_scalar_is_zero(&sec[rings - 1])) {
        return 0;
    }
    signs = &proof[len];
    /* We need one sign bit for each blinded value we send. */
    for (i = 0; i < (rings + 6) >> 3; i++) {
        signs[i] = 0;
        len++;
    }
    npub = 0;
    for (i = 0; i < rings; i++) {
        /*OPT: Use the precomputed gen2 basis?*/
        secp256k1_pedersen_ecmult(ecmult_gen_ctx, pedersen_ctx, &pubs[npub], &sec[i], ((uint64_t)secidx[i] * scale) << (i*2));
        if (secp256k1_gej_is_infinity(&pubs[npub])) {
            return 0;
        }
        if (i < rings - 1) {
            size_t size = 33;
            secp256k1_ge c;
            /*OPT: split loop and batch invert.*/
            secp256k1_ge_set_gej_var(&c, &pubs[npub]);
            if(!secp256k1_eckey_pubkey_serialize(&c, tmp, &size, 1)) {
                return 0;
            }
            secp256k1_sha256_write(&sha256_m, tmp, 33);
            signs[i>>3] |= (tmp[0] == 3) << (i&7);
            memcpy(&proof[len], &tmp[1], 32);
            len += 32;
        }
        npub += rsizes[i];
    }
    secp256k1_rangeproof_pub_expand(rangeproof_ctx, pubs, exp, rsizes, rings);
    secp256k1_sha256_finalize(&sha256_m, tmp);
    if (!secp256k1_borromean_sign(ecmult_ctx, ecmult_gen_ctx, &proof[len], s, pubs, k, sec, rsizes, secidx, rings, tmp, 32)) {
        return 0;
    }
    len += 32;
    for (i = 0; i < npub; i++) {
        secp256k1_scalar_get_b32(&proof[len],&s[i]);
        len += 32;
    }
    VERIFY_CHECK(len <= *plen);
    *plen = len;
    memset(prep, 0, 4096);
    return 1;
}

/* Computes blinding factor x given k, s, and the challenge e. */
SECP256K1_INLINE static void secp256k1_rangeproof_recover_x(secp256k1_scalar *x, const secp256k1_scalar *k, const secp256k1_scalar *e,
 const secp256k1_scalar *s) {
    secp256k1_scalar stmp;
    secp256k1_scalar_negate(x, s);
    secp256k1_scalar_add(x, x, k);
    secp256k1_scalar_inverse(&stmp, e);
    secp256k1_scalar_mul(x, x, &stmp);
}

/* Computes ring's nonce given the blinding factor x, the challenge e, and the signature s. */
SECP256K1_INLINE static void secp256k1_rangeproof_recover_k(secp256k1_scalar *k, const secp256k1_scalar *x, const secp256k1_scalar *e,
 const secp256k1_scalar *s) {
    secp256k1_scalar stmp;
    secp256k1_scalar_mul(&stmp, x, e);
    secp256k1_scalar_add(k, s, &stmp);
}

SECP256K1_INLINE static void secp256k1_rangeproof_ch32xor(unsigned char *x, const unsigned char *y) {
    int i;
    for (i = 0; i < 32; i++) {
        x[i] ^= y[i];
    }
}

SECP256K1_INLINE static int secp256k1_rangeproof_rewind_inner(secp256k1_scalar *blind, uint64_t *v,
 unsigned char *m, int *mlen, secp256k1_scalar *ev, secp256k1_scalar *s,
 int *rsizes, int rings, const unsigned char *nonce, const unsigned char *commit, const unsigned char *proof, int len) {
    secp256k1_scalar s_orig[128];
    secp256k1_scalar sec[32];
    secp256k1_scalar stmp;
    unsigned char prep[4096];
    unsigned char tmp[32];
    uint64_t value;
    int offset;
    int i;
    int j;
    int b;
    int skip1;
    int skip2;
    int npub;
    npub = ((rings - 1) << 2) + rsizes[rings-1];
    VERIFY_CHECK(npub <= 128);
    VERIFY_CHECK(npub >= 1);
    memset(prep, 0, 4096);
    /* Reconstruct the provers random values. */
    secp256k1_rangeproof_genrand(sec, s_orig, prep, rsizes, rings, nonce, commit, proof, len);
    *v = UINT64_MAX;
    secp256k1_scalar_clear(blind);
    if (rings == 1 && rsizes[0] == 1) {
        /* With only a single proof, we can only recover the blinding factor. */
        secp256k1_rangeproof_recover_x(blind, &s_orig[0], &ev[0], &s[0]);
        if (v) {
            *v = 0;
        }
        if (mlen) {
            *mlen = 0;
        }
        return 1;
    }
    npub = (rings - 1) << 2;
    for (j = 0; j < 2; j++) {
        int idx;
        /* Look for a value encoding in the last ring. */
        idx = npub + rsizes[rings - 1] - 1 - j;
        secp256k1_scalar_get_b32(tmp, &s[idx]);
        secp256k1_rangeproof_ch32xor(tmp, &prep[idx * 32]);
        if ((tmp[0] & 128) && (memcmp(&tmp[16], &tmp[24], 8) == 0) && (memcmp(&tmp[8], &tmp[16], 8) == 0)) {
            value = 0;
            for (i = 0; i < 8; i++) {
                value = (value << 8) + tmp[24 + i];
            }
            if (v) {
                *v = value;
            }
            memcpy(&prep[idx * 32], tmp, 32);
            break;
        }
    }
    if (j > 1) {
        /* Couldn't extract a value. */
        if (mlen) {
            *mlen = 0;
        }
        return 0;
    }
    skip1 = rsizes[rings - 1] - 1 - j;
    skip2 = ((value >> ((rings - 1) << 1)) & 3);
    if (skip1 == skip2) {
        /*Value is in wrong position.*/
        if (mlen) {
            *mlen = 0;
        }
        return 0;
    }
    skip1 += (rings - 1) << 2;
    skip2 += (rings - 1) << 2;
    /* Like in the rsize[] == 1 case, Having figured out which s is the one which was not forged, we can recover the blinding factor. */
    secp256k1_rangeproof_recover_x(&stmp, &s_orig[skip2], &ev[skip2], &s[skip2]);
    secp256k1_scalar_negate(&sec[rings - 1], &sec[rings - 1]);
    secp256k1_scalar_add(blind, &stmp, &sec[rings - 1]);
    if (!m || !mlen || *mlen == 0) {
        if (mlen) {
            *mlen = 0;
        }
        /* FIXME: cleanup in early out/failure cases. */
        return 1;
    }
    offset = 0;
    npub = 0;
    for (i = 0; i < rings; i++) {
        int idx;
        idx = (value >> (i << 1)) & 3;
        for (j = 0; j < rsizes[i]; j++) {
            if (npub == skip1 || npub == skip2) {
                npub++;
                continue;
            }
            if (idx == j) {
                /** For the non-forged signatures the signature is calculated instead of random, instead we recover the prover's nonces.
                 *  this could just as well recover the blinding factors and messages could be put there as is done for recovering the
                 *  blinding factor in the last ring, but it takes an inversion to recover x so it's faster to put the message data in k.
                 */
                secp256k1_rangeproof_recover_k(&stmp, &sec[i], &ev[npub], &s[npub]);
            } else {
                stmp = s[npub];
            }
            secp256k1_scalar_get_b32(tmp, &stmp);
            secp256k1_rangeproof_ch32xor(tmp, &prep[npub * 32]);
            for (b = 0; b < 32 && offset < *mlen; b++) {
                m[offset] = tmp[b];
                offset++;
            }
            npub++;
        }
    }
    *mlen = offset;
    memset(prep, 0, 4096);
    for (i = 0; i < 128; i++) {
        secp256k1_scalar_clear(&s_orig[i]);
    }
    for (i = 0; i < 32; i++) {
        secp256k1_scalar_clear(&sec[i]);
    }
    secp256k1_scalar_clear(&stmp);
    return 1;
}

SECP256K1_INLINE static int secp256k1_rangeproof_getheader_impl(int *offset, int *exp, int *mantissa, uint64_t *scale,
 uint64_t *min_value, uint64_t *max_value, const unsigned char *proof, int plen) {
    int i;
    int has_nz_range;
    int has_min;
    if (plen < 65 || ((proof[*offset] & 128) != 0)) {
        return 0;
    }
    has_nz_range = proof[*offset] & 64;
    has_min = proof[*offset] & 32;
    *exp = -1;
    *mantissa = 0;
    if (has_nz_range) {
        *exp = proof[*offset] & 31;
        *offset += 1;
        if (*exp > 18) {
           return 0;
        }
        *mantissa = proof[*offset] + 1;
        if (*mantissa > 64) {
            return 0;
         }
        *max_value = UINT64_MAX>>(64-*mantissa);
    } else {
        *max_value = 0;
    }
    *offset += 1;
    *scale = 1;
    for (i = 0; i < *exp; i++) {
        if (*max_value > UINT64_MAX / 10) {
            return 0;
        }
        *max_value *= 10;
        *scale *= 10;
    }
    *min_value = 0;
    if (has_min) {
        if(plen - *offset < 8) {
            return 0;
        }
        /*FIXME: Compact minvalue encoding?*/
        for (i = 0; i < 8; i++) {
            *min_value = (*min_value << 8) | proof[*offset + i];
        }
        *offset += 8;
    }
    if (*max_value > UINT64_MAX - *min_value) {
        return 0;
    }
    *max_value += *min_value;
    return 1;
}

/* Verifies range proof (len plen) for 33-byte commit, the min/max values proven are put in the min/max arguments; returns 0 on failure 1 on success.*/
SECP256K1_INLINE static int secp256k1_rangeproof_verify_impl(const secp256k1_ecmult_context* ecmult_ctx,
 const secp256k1_ecmult_gen_context* ecmult_gen_ctx,
 const secp256k1_pedersen_context* pedersen_ctx, const secp256k1_rangeproof_context* rangeproof_ctx,
 unsigned char *blindout, uint64_t *value_out, unsigned char *message_out, int *outlen, const unsigned char *nonce,
 uint64_t *min_value, uint64_t *max_value, const unsigned char *commit, const unsigned char *proof, int plen) {
    secp256k1_gej accj;
    secp256k1_gej pubs[128];
    secp256k1_ge c;
    secp256k1_scalar s[128];
    secp256k1_scalar evalues[128]; /* Challenges, only used during proof rewind. */
    secp256k1_sha256_t sha256_m;
    int rsizes[32];
    int ret;
    int i;
    size_t size;
    int exp;
    int mantissa;
    int offset;
    int rings;
    int overflow;
    int npub;
    int offset_post_header;
    uint64_t scale;
    unsigned char signs[31];
    unsigned char m[33];
    const unsigned char *e0;
    offset = 0;
    if (!secp256k1_rangeproof_getheader_impl(&offset, &exp, &mantissa, &scale, min_value, max_value, proof, plen)) {
        return 0;
    }
    offset_post_header = offset;
    rings = 1;
    rsizes[0] = 1;
    npub = 1;
    if (mantissa != 0) {
        rings = (mantissa >> 1);
        for (i = 0; i < rings; i++) {
            rsizes[i] = 4;
        }
        npub = (mantissa >> 1) << 2;
        if (mantissa & 1) {
            rsizes[rings] = 2;
            npub += rsizes[rings];
            rings++;
        }
    }
    VERIFY_CHECK(rings <= 32);
    if (plen - offset < 32 * (npub + rings - 1) + 32 + ((rings+6) >> 3)) {
        return 0;
    }
    secp256k1_sha256_initialize(&sha256_m);
    secp256k1_sha256_write(&sha256_m, commit, 33);
    secp256k1_sha256_write(&sha256_m, proof, offset);
    for(i = 0; i < rings - 1; i++) {
        signs[i] = (proof[offset + ( i>> 3)] & (1 << (i & 7))) != 0;
    }
    offset += (rings + 6) >> 3;
    if ((rings - 1) & 7) {
        /* Number of coded blinded points is not a multiple of 8, force extra sign bits to 0 to reject mutation. */
        if ((proof[offset - 1] >> ((rings - 1) & 7)) != 0) {
            return 0;
        }
    }
    npub = 0;
    secp256k1_gej_set_infinity(&accj);
    if (*min_value) {
        secp256k1_pedersen_ecmult_small(pedersen_ctx, &accj, *min_value);
    }
    for(i = 0; i < rings - 1; i++) {
        memcpy(&m[1], &proof[offset], 32);
        m[0] = 2 + signs[i];
        if (!secp256k1_eckey_pubkey_parse(&c, m, 33)) {
            return 0;
        }
        secp256k1_sha256_write(&sha256_m, m, 33);
        secp256k1_gej_set_ge(&pubs[npub], &c);
        secp256k1_gej_add_ge_var(&accj, &accj, &c, NULL);
        offset += 32;
        npub += rsizes[i];
    }
    secp256k1_gej_neg(&accj, &accj);
    if (!secp256k1_eckey_pubkey_parse(&c, commit, 33)) {
        return 0;
    }
    secp256k1_gej_add_ge_var(&pubs[npub], &accj, &c, NULL);
    if (secp256k1_gej_is_infinity(&pubs[npub])) {
        return 0;
    }
    secp256k1_rangeproof_pub_expand(rangeproof_ctx, pubs, exp, rsizes, rings);
    npub += rsizes[rings - 1];
    e0 = &proof[offset];
    offset += 32;
    for (i = 0; i < npub; i++) {
        secp256k1_scalar_set_b32(&s[i], &proof[offset], &overflow);
        if (overflow) {
            return 0;
        }
        offset += 32;
    }
    if (offset != plen) {
        /*Extra data found, reject.*/
        return 0;
    }
    secp256k1_sha256_finalize(&sha256_m, m);
    ret = secp256k1_borromean_verify(ecmult_ctx, nonce ? evalues : NULL, e0, s, pubs, rsizes, rings, m, 32);
    if (ret && nonce) {
        /* Given the nonce, try rewinding the witness to recover its initial state. */
        secp256k1_scalar blind;
        unsigned char commitrec[33];
        uint64_t vv;
        if (!ecmult_gen_ctx) {
            return 0;
        }
        if (!secp256k1_rangeproof_rewind_inner(&blind, &vv, message_out, outlen, evalues, s, rsizes, rings, nonce, commit, proof, offset_post_header)) {
            return 0;
        }
        /* Unwind apparently successful, see if the commitment can be reconstructed. */
        /* FIXME: should check vv is in the mantissa's range. */
        vv = (vv * scale) + *min_value;
        secp256k1_pedersen_ecmult(ecmult_gen_ctx, pedersen_ctx, &accj, &blind, vv);
        if (secp256k1_gej_is_infinity(&accj)) {
            return 0;
        }
        secp256k1_ge_set_gej(&c, &accj);
        size = 33;
        secp256k1_eckey_pubkey_serialize(&c, commitrec, &size, 1);
        if (size != 33 || memcmp(commitrec, commit, 33) != 0) {
            return 0;
        }
        if (blindout) {
            secp256k1_scalar_get_b32(blindout, &blind);
        }
        if (value_out) {
            *value_out = vv;
        }
    }
    return ret;
}

#endif
