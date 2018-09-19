#!/bin/bash

tagname=`git symbolic-ref -q --short HEAD || git describe --tags --exact-match`

if [[ $TRAVIS_OS_NAME == 'osx' ]]; then

    # Do some custom requirements on OS X
    cd target/release ; rm -f *.tgz; tar zcf "grin-$tagname#$TRAVIS_JOB_ID-osx.tgz" grin
    /bin/ls -ls *.tgz  | awk '{print $6,$7,$8,$9,$10}'
    cd - > /dev/null;
else
    # Do some custom requirements on Linux
    cd target/release ; rm -f *.tgz; tar zcf "grin-$tagname#$TRAVIS_JOB_ID-linux-amd64.tgz" grin
    /bin/ls -ls *.tgz  | awk '{print $6,$7,$8,$9,$10}'
    cd - > /dev/null;
fi
