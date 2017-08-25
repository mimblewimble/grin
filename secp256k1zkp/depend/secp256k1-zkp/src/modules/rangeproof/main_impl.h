/**********************************************************************
 * Copyright (c) 2014-2015 Gregory Maxwell                            *
 * Distributed under the MIT software license, see the accompanying   *
 * file COPYING or http://www.opensource.org/licenses/mit-license.php.*
 **********************************************************************/

#ifndef SECP256K1_MODULE_RANGEPROOF_MAIN
#define SECP256K1_MODULE_RANGEPROOF_MAIN

#include "modules/rangeproof/pedersen_impl.h"
#include "modules/rangeproof/borromean_impl.h"
#include "modules/rangeproof/rangeproof_impl.h"
#include "modules/rangeproof/switch_impl.h"

void secp256k1_switch_context_initialize(secp256k1_context* ctx) {
    secp256k1_pedersen_context_build(&ctx->switch_ctx, secp256k1_ge_const_g3, &ctx->error_callback);
}

/* Generates a simple switch commitment: *commit = blind * G3. The commitment is 33 bytes, the blinding factor is 32 bytes.*/
int secp256k1_switch_commit(const secp256k1_context* ctx, unsigned char *commit, unsigned char *blind) {
    secp256k1_gej rj;
    secp256k1_ge r;
    secp256k1_scalar sec;
    size_t sz;
    int overflow;
    int ret = 0;
    ARG_CHECK(ctx != NULL);
    ARG_CHECK(secp256k1_pedersen_context_is_built(&ctx->switch_ctx));
    ARG_CHECK(commit != NULL);
    ARG_CHECK(blind != NULL);
    secp256k1_scalar_set_b32(&sec, blind, &overflow);
    if (!overflow) {
        secp256k1_switch_ecmult(&ctx->switch_ctx, &rj, &sec);
        if (!secp256k1_gej_is_infinity(&rj)) {
            secp256k1_ge_set_gej(&r, &rj);
            sz = 33;
            ret = secp256k1_eckey_pubkey_serialize(&r, commit, &sz, 1);
        }
        secp256k1_gej_clear(&rj);
        secp256k1_ge_clear(&r);
    }
    secp256k1_scalar_clear(&sec);
    return ret;
}

void secp256k1_pedersen_context_initialize(secp256k1_context* ctx) {
    secp256k1_pedersen_context_build(&ctx->pedersen_ctx, secp256k1_ge_const_g2, &ctx->error_callback);
}

/* Generates a pedersen commitment: *commit = blind * G + value * G2. The commitment is 33 bytes, the blinding factor is 32 bytes.*/
int secp256k1_pedersen_commit(const secp256k1_context* ctx, unsigned char *commit, unsigned char *blind, uint64_t value) {
    secp256k1_gej rj;
    secp256k1_ge r;
    secp256k1_scalar sec;
    size_t sz;
    int overflow;
    int ret = 0;
    ARG_CHECK(ctx != NULL);
    ARG_CHECK(secp256k1_ecmult_gen_context_is_built(&ctx->ecmult_gen_ctx));
    ARG_CHECK(secp256k1_pedersen_context_is_built(&ctx->pedersen_ctx));
    ARG_CHECK(commit != NULL);
    ARG_CHECK(blind != NULL);
    secp256k1_scalar_set_b32(&sec, blind, &overflow);
    if (!overflow) {
        secp256k1_pedersen_ecmult(&ctx->ecmult_gen_ctx, &ctx->pedersen_ctx, &rj, &sec, value);
        if (!secp256k1_gej_is_infinity(&rj)) {
            secp256k1_ge_set_gej(&r, &rj);
            sz = 33;
            ret = secp256k1_eckey_pubkey_serialize(&r, commit, &sz, 1);
        }
        secp256k1_gej_clear(&rj);
        secp256k1_ge_clear(&r);
    }
    secp256k1_scalar_clear(&sec);
    return ret;
}

/** Takes a list of n pointers to 32 byte blinding values, the first negs of which are treated with positive sign and the rest
 *  negative, then calculates an additional blinding value that adds to zero.
 */
int secp256k1_pedersen_blind_sum(const secp256k1_context* ctx, unsigned char *blind_out, const unsigned char * const *blinds, int n, int npositive) {
    secp256k1_scalar acc;
    secp256k1_scalar x;
    int i;
    int overflow;
    ARG_CHECK(ctx != NULL);
    ARG_CHECK(blind_out != NULL);
    ARG_CHECK(blinds != NULL);
    secp256k1_scalar_set_int(&acc, 0);
    for (i = 0; i < n; i++) {
        secp256k1_scalar_set_b32(&x, blinds[i], &overflow);
        if (overflow) {
            return 0;
        }
        if (i >= npositive) {
            secp256k1_scalar_negate(&x, &x);
        }
        secp256k1_scalar_add(&acc, &acc, &x);
    }
    secp256k1_scalar_get_b32(blind_out, &acc);
    secp256k1_scalar_clear(&acc);
    secp256k1_scalar_clear(&x);
    return 1;
}

/* Takes two list of 33-byte commitments and sums the first set, subtracts the second and returns the resulting commitment. */
int secp256k1_pedersen_commit_sum(const secp256k1_context* ctx, unsigned char *commit_out,
 const unsigned char * const *commits, int pcnt, const unsigned char * const *ncommits, int ncnt) {
    secp256k1_gej accj;
    secp256k1_ge add;
    int i;
    int ret = 0;
    ARG_CHECK(ctx != NULL);
    ARG_CHECK(!pcnt || (commits != NULL));
    ARG_CHECK(!ncnt || (ncommits != NULL));
    ARG_CHECK(commit_out != NULL);
    ARG_CHECK(secp256k1_pedersen_context_is_built(&ctx->pedersen_ctx));
    secp256k1_gej_set_infinity(&accj);
    for (i = 0; i < ncnt; i++) {
        if (!secp256k1_eckey_pubkey_parse(&add, ncommits[i], 33)) {
            return 0;
        }
        secp256k1_gej_add_ge_var(&accj, &accj, &add, NULL);
    }
    secp256k1_gej_neg(&accj, &accj);
    for (i = 0; i < pcnt; i++) {
        if (!secp256k1_eckey_pubkey_parse(&add, commits[i], 33)) {
            return 0;
        }
        secp256k1_gej_add_ge_var(&accj, &accj, &add, NULL);
    }
    if (!secp256k1_gej_is_infinity(&accj)) {
        size_t sz = 33;
        secp256k1_ge acc;
        secp256k1_ge_set_gej(&acc, &accj);
        ret = secp256k1_eckey_pubkey_serialize(&acc, commit_out, &sz, 1);
    }

    return ret;
}

/* Takes two list of 33-byte commitments and sums the first set and subtracts the second and verifies that they sum to excess. */
int secp256k1_pedersen_verify_tally(const secp256k1_context* ctx, const unsigned char * const *commits, int pcnt,
 const unsigned char * const *ncommits, int ncnt, int64_t excess) {
    secp256k1_gej accj;
    secp256k1_ge add;
    int i;
    ARG_CHECK(ctx != NULL);
    ARG_CHECK(!pcnt || (commits != NULL));
    ARG_CHECK(!ncnt || (ncommits != NULL));
    ARG_CHECK(secp256k1_pedersen_context_is_built(&ctx->pedersen_ctx));
    secp256k1_gej_set_infinity(&accj);
    if (excess) {
        uint64_t ex;
        int neg;
        /* Take the absolute value, and negate the result if the input was negative. */
        neg = secp256k1_sign_and_abs64(&ex, excess);
        secp256k1_pedersen_ecmult_small(&ctx->pedersen_ctx, &accj, ex);
        if (neg) {
            secp256k1_gej_neg(&accj, &accj);
        }
    }
    for (i = 0; i < ncnt; i++) {
        if (!secp256k1_eckey_pubkey_parse(&add, ncommits[i], 33)) {
            return 0;
        }
        secp256k1_gej_add_ge_var(&accj, &accj, &add, NULL);
    }
    secp256k1_gej_neg(&accj, &accj);
    for (i = 0; i < pcnt; i++) {
        if (!secp256k1_eckey_pubkey_parse(&add, commits[i], 33)) {
            return 0;
        }
        secp256k1_gej_add_ge_var(&accj, &accj, &add, NULL);
    }
    return secp256k1_gej_is_infinity(&accj);
}

void secp256k1_rangeproof_context_initialize(secp256k1_context* ctx) {
    secp256k1_rangeproof_context_build(&ctx->rangeproof_ctx, &ctx->error_callback);
}

int secp256k1_rangeproof_info(const secp256k1_context* ctx, int *exp, int *mantissa,
 uint64_t *min_value, uint64_t *max_value, const unsigned char *proof, int plen) {
    int offset;
    uint64_t scale;
    ARG_CHECK(exp != NULL);
    ARG_CHECK(mantissa != NULL);
    ARG_CHECK(min_value != NULL);
    ARG_CHECK(max_value != NULL);
    offset = 0;
    scale = 1;
    (void)ctx;
    return secp256k1_rangeproof_getheader_impl(&offset, exp, mantissa, &scale, min_value, max_value, proof, plen);
}

int secp256k1_rangeproof_rewind(const secp256k1_context* ctx,
 unsigned char *blind_out, uint64_t *value_out, unsigned char *message_out, int *outlen, const unsigned char *nonce,
 uint64_t *min_value, uint64_t *max_value,
 const unsigned char *commit, const unsigned char *proof, int plen) {
    ARG_CHECK(ctx != NULL);
    ARG_CHECK(commit != NULL);
    ARG_CHECK(proof != NULL);
    ARG_CHECK(min_value != NULL);
    ARG_CHECK(max_value != NULL);
    ARG_CHECK(secp256k1_ecmult_context_is_built(&ctx->ecmult_ctx));
    ARG_CHECK(secp256k1_ecmult_gen_context_is_built(&ctx->ecmult_gen_ctx));
    ARG_CHECK(secp256k1_pedersen_context_is_built(&ctx->pedersen_ctx));
    ARG_CHECK(secp256k1_rangeproof_context_is_built(&ctx->rangeproof_ctx));
    return secp256k1_rangeproof_verify_impl(&ctx->ecmult_ctx, &ctx->ecmult_gen_ctx, &ctx->pedersen_ctx, &ctx->rangeproof_ctx,
     blind_out, value_out, message_out, outlen, nonce, min_value, max_value, commit, proof, plen);
}

int secp256k1_rangeproof_verify(const secp256k1_context* ctx, uint64_t *min_value, uint64_t *max_value,
 const unsigned char *commit, const unsigned char *proof, int plen) {
    ARG_CHECK(ctx != NULL);
    ARG_CHECK(commit != NULL);
    ARG_CHECK(proof != NULL);
    ARG_CHECK(min_value != NULL);
    ARG_CHECK(max_value != NULL);
    ARG_CHECK(secp256k1_ecmult_context_is_built(&ctx->ecmult_ctx));
    ARG_CHECK(secp256k1_pedersen_context_is_built(&ctx->pedersen_ctx));
    ARG_CHECK(secp256k1_rangeproof_context_is_built(&ctx->rangeproof_ctx));
    return secp256k1_rangeproof_verify_impl(&ctx->ecmult_ctx, NULL, &ctx->pedersen_ctx, &ctx->rangeproof_ctx,
     NULL, NULL, NULL, NULL, NULL, min_value, max_value, commit, proof, plen);
}

int secp256k1_rangeproof_sign(const secp256k1_context* ctx, unsigned char *proof, int *plen, uint64_t min_value,
 const unsigned char *commit, const unsigned char *blind, const unsigned char *nonce, int exp, int min_bits, uint64_t value){
    ARG_CHECK(ctx != NULL);
    ARG_CHECK(proof != NULL);
    ARG_CHECK(plen != NULL);
    ARG_CHECK(commit != NULL);
    ARG_CHECK(blind != NULL);
    ARG_CHECK(nonce != NULL);
    ARG_CHECK(secp256k1_ecmult_context_is_built(&ctx->ecmult_ctx));
    ARG_CHECK(secp256k1_ecmult_gen_context_is_built(&ctx->ecmult_gen_ctx));
    ARG_CHECK(secp256k1_pedersen_context_is_built(&ctx->pedersen_ctx));
    ARG_CHECK(secp256k1_rangeproof_context_is_built(&ctx->rangeproof_ctx));
    return secp256k1_rangeproof_sign_impl(&ctx->ecmult_ctx, &ctx->ecmult_gen_ctx, &ctx->pedersen_ctx, &ctx->rangeproof_ctx,
     proof, plen, min_value, commit, blind, nonce, exp, min_bits, value);
}

#endif
