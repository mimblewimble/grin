# Contributing

Find an area you can help with and do it. Open source is about collaboration and open participation. Try to make your code look like what already exists and submit a pull request.

The [list of issues](https://github.com/mimblewimble/grin/issues) is a good place to start, especially the ones tagged as "help wanted" (but don't let that stop you from looking at others). If you're looking for additional ideas, the code includes `TODO` comments for minor to major improvements. Grep is your friend.

Additional tests are rewarded with an immense amount of positive karma.

More documentation or updates/fixes to existing documentation are also very welcome. However, if submitting a PR consisting of documentation changes only, please try to ensure that the change is significantly more substantial than one or two lines. For example, working through an install document and making changes and updates throughout as you find issues is worth a PR. For typos and other small changes, either contact one of the developers, or if you think it's a significant enough error to cause problems for other users, please feel free to open an issue.

Find us:

* Chat: [Gitter](https://gitter.im/grin_community/Lobby).
* Mailing list: join the [~MimbleWimble team](https://launchpad.net/~mimblewimble) and subscribe on Launchpad.

# Grin Style Guide

Grin uses `rustfmt` to maintain consistent formatting.
Please use the git commit hook as explained below.

## Install rustfmt

You should use rustup. See [build docs](doc/build.md) for more info.

```
rustup component add rustfmt-preview
rustup update
rustfmt --version
```

and verify you did get version `0.3.4-nightly (6714a44 2017-12-23)` or newer.

## Install git pre-commit hook

There is a basic git [pre-commit](../.hooks/pre-commit) hook in the repo.

The pre-commit hook will not prevent commits if style issues are present but it will
indicate any files that need formatting.

To enable this, create a symlink in `.git/hooks` (note the relative path) -

```
cd .git/hooks
ln -s -f ../../.hooks/pre-commit
```

## Running rustfmt

You can run rustfmt (i.e. rustfmt-preview) on one file or on all files.

First try a dry-run on a file you've worked on, say:
`rustfmt --write-mode diff -- client.rs`

Any errors or rustfmt failures? Fix those manually.

Then let rustfmt make any further changes and save you the work:

`rustfmt --write-mode overwrite -- client.rs`

*Please add the rustfmt corrections as a separate commit at the end of your Pull Request to make the reviewers happy.*


And don't use ~~`cargo +nightly fmt`~~ if at all possible.
