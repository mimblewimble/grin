/**********************************************************************
 * Copyright (c) 2015 Gregory Maxwell                                 *
 * Distributed under the MIT software license, see the accompanying   *
 * file COPYING or http://www.opensource.org/licenses/mit-license.php.*
 **********************************************************************/

#ifndef SECP256K1_MODULE_RANGEPROOF_TESTS
#define SECP256K1_MODULE_RANGEPROOF_TESTS

#include "include/secp256k1_rangeproof.h"

void test_pedersen(void) {
    unsigned char commits[33*19];
    const unsigned char *cptr[19];
    unsigned char blinds[32*19];
    const unsigned char *bptr[19];
    secp256k1_scalar s;
    uint64_t values[19];
    int64_t totalv;
    int i;
    int inputs;
    int outputs;
    int total;
    inputs = (secp256k1_rand32() & 7) + 1;
    outputs = (secp256k1_rand32() & 7) + 2;
    total = inputs + outputs;
    for (i = 0; i < 19; i++) {
        cptr[i] = &commits[i * 33];
        bptr[i] = &blinds[i * 32];
    }
    totalv = 0;
    for (i = 0; i < inputs; i++) {
        values[i] = secp256k1_rands64(0, INT64_MAX - totalv);
        totalv += values[i];
    }
    if (secp256k1_rand32() & 1) {
        for (i = 0; i < outputs; i++) {
            int64_t max = INT64_MAX;
            if (totalv < 0) {
                max += totalv;
            }
            values[i + inputs] = secp256k1_rands64(0, max);
            totalv -= values[i + inputs];
        }
    } else {
        for (i = 0; i < outputs - 1; i++) {
            values[i + inputs] = secp256k1_rands64(0, totalv);
            totalv -= values[i + inputs];
        }
        values[total - 1] = totalv >> (secp256k1_rand32() & 1);
        totalv -= values[total - 1];
    }
    for (i = 0; i < total - 1; i++) {
        random_scalar_order(&s);
        secp256k1_scalar_get_b32(&blinds[i * 32], &s);
    }
    CHECK(secp256k1_pedersen_blind_sum(ctx, &blinds[(total - 1) * 32], bptr, total - 1, inputs));
    for (i = 0; i < total; i++) {
        CHECK(secp256k1_pedersen_commit(ctx, &commits[i * 33], &blinds[i * 32], values[i]));
    }
    CHECK(secp256k1_pedersen_verify_tally(ctx, cptr, inputs, &cptr[inputs], outputs, totalv));
    CHECK(!secp256k1_pedersen_verify_tally(ctx, cptr, inputs, &cptr[inputs], outputs, totalv + 1));
    random_scalar_order(&s);
    for (i = 0; i < 4; i++) {
        secp256k1_scalar_get_b32(&blinds[i * 32], &s);
    }
    values[0] = INT64_MAX;
    values[1] = 0;
    values[2] = 1;
    for (i = 0; i < 3; i++) {
        CHECK(secp256k1_pedersen_commit(ctx, &commits[i * 33], &blinds[i * 32], values[i]));
    }
    CHECK(secp256k1_pedersen_verify_tally(ctx, &cptr[1], 1, &cptr[2], 1, -1));
    CHECK(secp256k1_pedersen_verify_tally(ctx, &cptr[2], 1, &cptr[1], 1, 1));
    CHECK(secp256k1_pedersen_verify_tally(ctx, &cptr[0], 1, &cptr[0], 1, 0));
    CHECK(secp256k1_pedersen_verify_tally(ctx, &cptr[0], 1, &cptr[1], 1, INT64_MAX));
    CHECK(secp256k1_pedersen_verify_tally(ctx, &cptr[1], 1, &cptr[1], 1, 0));
    CHECK(secp256k1_pedersen_verify_tally(ctx, &cptr[1], 1, &cptr[0], 1, -INT64_MAX));
}

void test_borromean(void) {
    unsigned char e0[32];
    secp256k1_scalar s[64];
    secp256k1_gej pubs[64];
    secp256k1_scalar k[8];
    secp256k1_scalar sec[8];
    secp256k1_ge ge;
    secp256k1_scalar one;
    unsigned char m[32];
    int rsizes[8];
    int secidx[8];
    int nrings;
    int i;
    int j;
    int c;
    secp256k1_rand256_test(m);
    nrings = 1 + (secp256k1_rand32()&7);
    c = 0;
    secp256k1_scalar_set_int(&one, 1);
    if (secp256k1_rand32()&1) {
        secp256k1_scalar_negate(&one, &one);
    }
    for (i = 0; i < nrings; i++) {
        rsizes[i] = 1 + (secp256k1_rand32()&7);
        secidx[i] = secp256k1_rand32() % rsizes[i];
        random_scalar_order(&sec[i]);
        random_scalar_order(&k[i]);
        if(secp256k1_rand32()&7) {
            sec[i] = one;
        }
        if(secp256k1_rand32()&7) {
            k[i] = one;
        }
        for (j = 0; j < rsizes[i]; j++) {
            random_scalar_order(&s[c + j]);
            if(secp256k1_rand32()&7) {
                s[i] = one;
            }
            if (j == secidx[i]) {
                secp256k1_ecmult_gen(&ctx->ecmult_gen_ctx, &pubs[c + j], &sec[i]);
            } else {
                random_group_element_test(&ge);
                random_group_element_jacobian_test(&pubs[c + j],&ge);
            }
        }
        c += rsizes[i];
    }
    CHECK(secp256k1_borromean_sign(&ctx->ecmult_ctx, &ctx->ecmult_gen_ctx, e0, s, pubs, k, sec, rsizes, secidx, nrings, m, 32));
    CHECK(secp256k1_borromean_verify(&ctx->ecmult_ctx, NULL, e0, s, pubs, rsizes, nrings, m, 32));
    i = secp256k1_rand32() % c;
    secp256k1_scalar_negate(&s[i],&s[i]);
    CHECK(!secp256k1_borromean_verify(&ctx->ecmult_ctx, NULL, e0, s, pubs, rsizes, nrings, m, 32));
    secp256k1_scalar_negate(&s[i],&s[i]);
    secp256k1_scalar_set_int(&one, 1);
    for(j = 0; j < 4; j++) {
        i = secp256k1_rand32() % c;
        if (secp256k1_rand32() & 1) {
            secp256k1_gej_double_var(&pubs[i],&pubs[i], NULL);
        } else {
            secp256k1_scalar_add(&s[i],&s[i],&one);
        }
        CHECK(!secp256k1_borromean_verify(&ctx->ecmult_ctx, NULL, e0, s, pubs, rsizes, nrings, m, 32));
    }
}

void test_rangeproof(void) {
    const uint64_t testvs[11] = {0, 1, 5, 11, 65535, 65537, INT32_MAX, UINT32_MAX, INT64_MAX - 1, INT64_MAX, UINT64_MAX};
    unsigned char commit[33];
    unsigned char commit2[33];
    unsigned char proof[5134];
    unsigned char blind[32];
    unsigned char blindout[32];
    unsigned char message[4096];
    int mlen;
    uint64_t v;
    uint64_t vout;
    uint64_t vmin;
    uint64_t minv;
    uint64_t maxv;
    int len;
    int i;
    int j;
    int k;
    secp256k1_rand256(blind);
    for (i = 0; i < 11; i++) {
        v = testvs[i];
        CHECK(secp256k1_pedersen_commit(ctx, commit, blind, v));
        for (vmin = 0; vmin < (i<9 && i > 0 ? 2 : 1); vmin++) {
            len = 5134;
            CHECK(secp256k1_rangeproof_sign(ctx, proof, &len, vmin, commit, blind, commit, 0, 0, v));
            CHECK(len <= 5134);
            mlen = 4096;
            CHECK(secp256k1_rangeproof_rewind(ctx, blindout, &vout, message, &mlen, commit, &minv, &maxv, commit, proof, len));
            for (j = 0; j < mlen; j++) {
                CHECK(message[j] == 0);
            }
            CHECK(mlen <= 4096);
            CHECK(memcmp(blindout, blind, 32) == 0);
            CHECK(vout == v);
            CHECK(minv <= v);
            CHECK(maxv >= v);
            len = 5134;
            CHECK(secp256k1_rangeproof_sign(ctx, proof, &len, v, commit, blind, commit, -1, 64, v));
            CHECK(len <= 73);
            CHECK(secp256k1_rangeproof_rewind(ctx, blindout, &vout, NULL, NULL, commit, &minv, &maxv, commit, proof, len));
            CHECK(memcmp(blindout, blind, 32) == 0);
            CHECK(vout == v);
            CHECK(minv == v);
            CHECK(maxv == v);
        }
    }
    secp256k1_rand256(blind);
    v = INT64_MAX - 1;
    CHECK(secp256k1_pedersen_commit(ctx, commit, blind, v));
    for (i = 0; i < 19; i++) {
        len = 5134;
        CHECK(secp256k1_rangeproof_sign(ctx, proof, &len, 0, commit, blind, commit, i, 0, v));
        CHECK(secp256k1_rangeproof_verify(ctx, &minv, &maxv, commit, proof, len));
        CHECK(len <= 5134);
        CHECK(minv <= v);
        CHECK(maxv >= v);
    }
    secp256k1_rand256(blind);
    {
        /*Malleability test.*/
        v = secp256k1_rands64(0, 255);
        CHECK(secp256k1_pedersen_commit(ctx, commit, blind, v));
        len = 5134;
        CHECK(secp256k1_rangeproof_sign(ctx, proof, &len, 0, commit, blind, commit, 0, 3, v));
        CHECK(len <= 5134);
        for (i = 0; i < len*8; i++) {
            proof[i >> 3] ^= 1 << (i & 7);
            CHECK(!secp256k1_rangeproof_verify(ctx, &minv, &maxv, commit, proof, len));
            proof[i >> 3] ^= 1 << (i & 7);
        }
        CHECK(secp256k1_rangeproof_verify(ctx, &minv, &maxv, commit, proof, len));
        CHECK(minv <= v);
        CHECK(maxv >= v);
    }
    memcpy(commit2, commit, 33);
    for (i = 0; i < 10 * count; i++) {
        int exp;
        int min_bits;
        v = secp256k1_rands64(0, UINT64_MAX >> (secp256k1_rand32()&63));
        vmin = 0;
        if ((v < INT64_MAX) && (secp256k1_rand32()&1)) {
            vmin = secp256k1_rands64(0, v);
        }
        secp256k1_rand256(blind);
        CHECK(secp256k1_pedersen_commit(ctx, commit, blind, v));
        len = 5134;
        exp = (int)secp256k1_rands64(0,18)-(int)secp256k1_rands64(0,18);
        if (exp < 0) {
            exp = -exp;
        }
        min_bits = (int)secp256k1_rands64(0,64)-(int)secp256k1_rands64(0,64);
        if (min_bits < 0) {
            min_bits = -min_bits;
        }
        CHECK(secp256k1_rangeproof_sign(ctx, proof, &len, vmin, commit, blind, commit, exp, min_bits, v));
        CHECK(len <= 5134);
        mlen = 4096;
        CHECK(secp256k1_rangeproof_rewind(ctx, blindout, &vout, message, &mlen, commit, &minv, &maxv, commit, proof, len));
        for (j = 0; j < mlen; j++) {
            CHECK(message[j] == 0);
        }
        CHECK(mlen <= 4096);
        CHECK(memcmp(blindout, blind, 32) == 0);
        CHECK(vout == v);
        CHECK(minv <= v);
        CHECK(maxv >= v);
        CHECK(secp256k1_rangeproof_rewind(ctx, blindout, &vout, NULL, NULL, commit, &minv, &maxv, commit, proof, len));
        memcpy(commit2, commit, 33);
    }
    for (j = 0; j < 10; j++) {
        for (i = 0; i < 96; i++) {
            secp256k1_rand256(&proof[i * 32]);
        }
        for (k = 0; k < 128; k++) {
            len = k;
            CHECK(!secp256k1_rangeproof_verify(ctx, &minv, &maxv, commit2, proof, len));
        }
        len = secp256k1_rands64(0, 3072);
        CHECK(!secp256k1_rangeproof_verify(ctx, &minv, &maxv, commit2, proof, len));
    }
}

void run_rangeproof_tests(void) {
    int i;
    secp256k1_pedersen_context_initialize(ctx);
    secp256k1_rangeproof_context_initialize(ctx);
    for (i = 0; i < 10*count; i++) {
        test_pedersen();
    }
    for (i = 0; i < 10*count; i++) {
        test_borromean();
    }
    test_rangeproof();
}

#endif
