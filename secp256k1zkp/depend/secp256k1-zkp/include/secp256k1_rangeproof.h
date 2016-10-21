#ifndef _SECP256K1_RANGEPROOF_
# define _SECP256K1_RANGEPROOF_

# include "secp256k1.h"

# ifdef __cplusplus
extern "C" {
# endif

#include <stdint.h>

/** Initialize a context for usage with Pedersen commitments. */
void secp256k1_pedersen_context_initialize(secp256k1_context* ctx);

/** Generate a pedersen commitment.
 *  Returns 1: commitment successfully created.
 *          0: error
 *  In:     ctx:        pointer to a context object, initialized for signing and Pedersen commitment (cannot be NULL)
 *          blind:      pointer to a 32-byte blinding factor (cannot be NULL)
 *          value:      unsigned 64-bit integer value to commit to.
 *  Out:    commit:     pointer to a 33-byte array for the commitment (cannot be NULL)
 *
 *  Blinding factors can be generated and verified in the same way as secp256k1 private keys for ECDSA.
 */
SECP256K1_WARN_UNUSED_RESULT int secp256k1_pedersen_commit(
  const secp256k1_context* ctx,
  unsigned char *commit,
  unsigned char *blind,
  uint64_t value
) SECP256K1_ARG_NONNULL(1) SECP256K1_ARG_NONNULL(2) SECP256K1_ARG_NONNULL(3);

/** Computes the sum of multiple positive and negative blinding factors.
 *  Returns 1: sum successfully computed.
 *          0: error
 *  In:     ctx:        pointer to a context object (cannot be NULL)
 *          blinds:     pointer to pointers to 32-byte character arrays for blinding factors. (cannot be NULL)
 *          n:          number of factors pointed to by blinds.
 *          nneg:       how many of the initial factors should be treated with a positive sign.
 *  Out:    blind_out:  pointer to a 32-byte array for the sum (cannot be NULL)
 */
SECP256K1_WARN_UNUSED_RESULT int secp256k1_pedersen_blind_sum(
  const secp256k1_context* ctx,
  unsigned char *blind_out,
  const unsigned char * const *blinds,
  int n,
  int npositive
) SECP256K1_ARG_NONNULL(1) SECP256K1_ARG_NONNULL(2) SECP256K1_ARG_NONNULL(3);

/** Computes the sum of multiple positive and negative pedersen commitments
 * Returns 1: sum successfully computed.
 * In:     ctx:        pointer to a context object, initialized for Pedersen commitment (cannot be NULL)
 *         commits:    pointer to pointers to 33-byte character arrays for the commitments. (cannot be NULL if pcnt is non-zero)
 *         pcnt:       number of commitments pointed to by commits.
 *         ncommits:   pointer to pointers to 33-byte character arrays for negative commitments. (cannot be NULL if ncnt is non-zero)
 *         ncnt:       number of commitments pointed to by ncommits.
 *  Out:   commit_out:  pointer to a 33-byte array for the sum (cannot be NULL)
 */
SECP256K1_WARN_UNUSED_RESULT int secp256k1_pedersen_commit_sum(
  const secp256k1_context* ctx,
  unsigned char *commit_out,
  const unsigned char * const *commits,
  int pcnt,
  const unsigned char * const *ncommits,
  int ncnt
) SECP256K1_ARG_NONNULL(1) SECP256K1_ARG_NONNULL(2) SECP256K1_ARG_NONNULL(3) SECP256K1_ARG_NONNULL(4);

/** Verify a tally of pedersen commitments
 * Returns 1: commitments successfully sum to zero.
 *         0: Commitments do not sum to zero or other error.
 * In:     ctx:        pointer to a context object, initialized for Pedersen commitment (cannot be NULL)
 *         commits:    pointer to pointers to 33-byte character arrays for the commitments. (cannot be NULL if pcnt is non-zero)
 *         pcnt:       number of commitments pointed to by commits.
 *         ncommits:   pointer to pointers to 33-byte character arrays for negative commitments. (cannot be NULL if ncnt is non-zero)
 *         ncnt:       number of commitments pointed to by ncommits.
 *         excess:     signed 64bit amount to add to the total to bring it to zero, can be negative.
 *
 * This computes sum(commit[0..pcnt)) - sum(ncommit[0..ncnt)) - excess*H == 0.
 *
 * A pedersen commitment is xG + vH where G and H are generators for the secp256k1 group and x is a blinding factor,
 * while v is the committed value. For a collection of commitments to sum to zero both their blinding factors and
 * values must sum to zero.
 *
 */
SECP256K1_WARN_UNUSED_RESULT int secp256k1_pedersen_verify_tally(
  const secp256k1_context* ctx,
  const unsigned char * const *commits,
  int pcnt,
  const unsigned char * const *ncommits,
  int ncnt,
  int64_t excess
) SECP256K1_ARG_NONNULL(1) SECP256K1_ARG_NONNULL(2) SECP256K1_ARG_NONNULL(4);

/** Initialize a context for usage with Pedersen commitments. */
void secp256k1_rangeproof_context_initialize(secp256k1_context* ctx);

/** Verify a proof that a committed value is within a range.
 * Returns 1: Value is within the range [0..2^64), the specifically proven range is in the min/max value outputs.
 *         0: Proof failed or other error.
 * In:   ctx: pointer to a context object, initialized for range-proof and commitment (cannot be NULL)
 *       commit: the 33-byte commitment being proved. (cannot be NULL)
 *       proof: pointer to character array with the proof. (cannot be NULL)
 *       plen: length of proof in bytes.
 * Out:  min_value: pointer to a unsigned int64 which will be updated with the minimum value that commit could have. (cannot be NULL)
 *       max_value: pointer to a unsigned int64 which will be updated with the maximum value that commit could have. (cannot be NULL)
 */
SECP256K1_WARN_UNUSED_RESULT int secp256k1_rangeproof_verify(
  const secp256k1_context* ctx,
  uint64_t *min_value,
  uint64_t *max_value,
  const unsigned char *commit,
  const unsigned char *proof,
  int plen
) SECP256K1_ARG_NONNULL(1) SECP256K1_ARG_NONNULL(2) SECP256K1_ARG_NONNULL(3) SECP256K1_ARG_NONNULL(4) SECP256K1_ARG_NONNULL(5);

/** Verify a range proof proof and rewind the proof to recover information sent by its author.
 *  Returns 1: Value is within the range [0..2^64), the specifically proven range is in the min/max value outputs, and the value and blinding were recovered.
 *          0: Proof failed, rewind failed, or other error.
 *  In:   ctx: pointer to a context object, initialized for range-proof and Pedersen commitment (cannot be NULL)
 *        commit: the 33-byte commitment being proved. (cannot be NULL)
 *        proof: pointer to character array with the proof. (cannot be NULL)
 *        plen: length of proof in bytes.
 *        nonce: 32-byte secret nonce used by the prover (cannot be NULL)
 *  In/Out: blind_out: storage for the 32-byte blinding factor used for the commitment
 *        value_out: pointer to an unsigned int64 which has the exact value of the commitment.
 *        message_out: pointer to a 4096 byte character array to receive message data from the proof author.
 *        outlen:  length of message data written to message_out.
 *        min_value: pointer to an unsigned int64 which will be updated with the minimum value that commit could have. (cannot be NULL)
 *        max_value: pointer to an unsigned int64 which will be updated with the maximum value that commit could have. (cannot be NULL)
 */
SECP256K1_WARN_UNUSED_RESULT int secp256k1_rangeproof_rewind(
  const secp256k1_context* ctx,
  unsigned char *blind_out,
  uint64_t *value_out,
  unsigned char *message_out,
  int *outlen,
  const unsigned char *nonce,
  uint64_t *min_value,
  uint64_t *max_value,
  const unsigned char *commit,
  const unsigned char *proof,
  int plen
) SECP256K1_ARG_NONNULL(1) SECP256K1_ARG_NONNULL(6) SECP256K1_ARG_NONNULL(7) SECP256K1_ARG_NONNULL(8) SECP256K1_ARG_NONNULL(9) SECP256K1_ARG_NONNULL(10);

/** Author a proof that a committed value is within a range.
 *  Returns 1: Proof successfully created.
 *          0: Error
 *  In:     ctx:    pointer to a context object, initialized for range-proof, signing, and Pedersen commitment (cannot be NULL)
 *          proof:  pointer to array to receive the proof, can be up to 5134 bytes. (cannot be NULL)
 *          min_value: constructs a proof where the verifer can tell the minimum value is at least the specified amount.
 *          commit: 33-byte array with the commitment being proved.
 *          blind:  32-byte blinding factor used by commit.
 *          nonce:  32-byte secret nonce used to initialize the proof (value can be reverse-engineered out of the proof if this secret is known.)
 *          exp:    Base-10 exponent. Digits below above will be made public, but the proof will be made smaller. Allowed range is -1 to 18.
 *                  (-1 is a special case that makes the value public. 0 is the most private.)
 *          min_bits: Number of bits of the value to keep private. (0 = auto/minimal, - 64).
 *          value:  Actual value of the commitment.
 *  In/out: plen:   point to an integer with the size of the proof buffer and the size of the constructed proof.
 *
 *  If min_value or exp is non-zero then the value must be on the range [0, 2^63) to prevent the proof range from spanning past 2^64.
 *
 *  If exp is -1 the value is revealed by the proof (e.g. it proves that the proof is a blinding of a specific value, without revealing the blinding key.)
 *
 *  This can randomly fail with probability around one in 2^100. If this happens, buy a lottery ticket and retry with a different nonce or blinding.
 *
 */
SECP256K1_WARN_UNUSED_RESULT int secp256k1_rangeproof_sign(
  const secp256k1_context* ctx,
  unsigned char *proof,
  int *plen,
  uint64_t min_value,
  const unsigned char *commit,
  const unsigned char *blind,
  const unsigned char *nonce,
  int exp,
  int min_bits,
  uint64_t value
) SECP256K1_ARG_NONNULL(1) SECP256K1_ARG_NONNULL(2) SECP256K1_ARG_NONNULL(3) SECP256K1_ARG_NONNULL(5) SECP256K1_ARG_NONNULL(6) SECP256K1_ARG_NONNULL(7);

/** Extract some basic information from a range-proof.
 *  Returns 1: Information successfully extracted.
 *          0: Decode failed.
 *  In:   ctx: pointer to a context object
 *        proof: pointer to character array with the proof.
 *        plen: length of proof in bytes.
 *  Out:  exp: Exponent used in the proof (-1 means the value isn't private).
 *        mantissa: Number of bits covered by the proof.
 *        min_value: pointer to an unsigned int64 which will be updated with the minimum value that commit could have. (cannot be NULL)
 *        max_value: pointer to an unsigned int64 which will be updated with the maximum value that commit could have. (cannot be NULL)
 */
SECP256K1_WARN_UNUSED_RESULT int secp256k1_rangeproof_info(
  const secp256k1_context* ctx,
  int *exp,
  int *mantissa,
  uint64_t *min_value,
  uint64_t *max_value,
  const unsigned char *proof,
  int plen
) SECP256K1_ARG_NONNULL(1) SECP256K1_ARG_NONNULL(2) SECP256K1_ARG_NONNULL(3) SECP256K1_ARG_NONNULL(4) SECP256K1_ARG_NONNULL(5);

# ifdef __cplusplus
}
# endif

#endif
