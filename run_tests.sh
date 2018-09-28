#!/bin/bash
#------------------------------------------------------------------
# This is the test script for runing all tests on local machine.
# Travis-CI tests are similar to this, except for dual platform.
#
# We can use this to reproduce Travis-CI failure locally, in case
# there's any true failure test cases.
#
# Usage:
#   ./run_tests.sh
#------------------------------------------------------------------

abort()
{
    printf >&2 "*** ABORTED ***\n"
    printf "An error occurred. Exiting...\n" >&2

    if [[ $ulimit < 512 ]]; then
        sudo ulimit -n $ulimit
        printf "Info: Your ulimit has been restored to $ulimit \n"
    fi
    exit 1
}

trap 'abort' 0

set -e

# Add your script below....
# If an error occurs, the abort() function will be called.
#----------------------------------------------------------
# ===> Your script starts here

ulimit=`ulimit -n 2>/dev/null`
if [[ $ulimit < 512 ]]; then
    printf "Info: Your ulimit $ulimit is not enough for this test. You need 'sudo ulimit -n 512' to continue this test... \n"
    sudo ulimit -n 512
    printf "Info: Your ulimit has been adjusted to 512 for this test. It will change back automatically when tests finish. \n"
fi

printf "Start testing on module: servers \n"
cd servers && rm -rf target/tmp && RUST_TEST_THREADS=1 cargo test --release; cd - > /dev/null;
printf "Test done for module: servers \n"

DIRS=(store chain pool wallet p2p api keychain core util config)
for TEST_DIR in $DIRS; do 
    printf "Start testing on module: $TEST_DIR \n"
    cd $TEST_DIR && rm -rf target/tmp && cargo test --release; cd - > /dev/null;
    printf "Test done for module: $TEST_DIR \n"
done

#----------------------------------------------------------
# ===> Your script ends here
trap : 0

printf >&2 "*** DONE *** \n"

if [[ $ulimit < 512 ]]; then
    ulimit -n $ulimit
    printf "Info: Your ulimit has been restored to $ulimit \n"
fi

