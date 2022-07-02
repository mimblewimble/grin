# Contributing

Find an area you can help with and do it. Open source is about collaboration and open participation. Try to make your code look like what already exists and submit a pull request.

The [list of issues](https://github.com/mimblewimble/grin/issues) is a good place to start, especially the ones tagged as "help wanted" (but don't let that stop you from looking at others). If you're looking for additional ideas, the code includes `TODO` comments for minor to major improvements. Use _Search in files_ in your code editor, or `grep "TODO" -r --exclude-dir=target --exclude-dir=.git .`.

Additional tests are rewarded with an immense amount of positive karma.

More documentation or updates/fixes to existing documentation are also very welcome.

# PR Guidelines

We generally prefer you to PR your work earlier rather than later. This ensures everyone else has a better idea of what's being worked on, and can help reduce wasted effort. If work on your PR has just begun, please feel free to create the PR with [WIP] (work in progress) in the PR title, and let us know when it's ready for review in the comments.

Since mainnet has been released, the bar for having PRs accepted has been raised. Before submitting your PR for approval, please be ensure it:
* Includes a proper description of what problems the PR addresses, as well as a detailed explanation as to what it changes
* Explains whether/how the change is consensus breaking or breaks existing client functionality
* Contains unit tests exercising new/changed functionality
* Fully considers the potential impact of the change on other parts of the system
* Describes how you've tested the change (e.g. against Testnet, etc)
* Updates any documentation that's affected by the PR

If submitting a PR consisting of documentation changes only, please try to ensure that the change is significantly more substantial than one or two lines. For example, working through an install document and making changes and updates throughout as you find issues is worth a PR. For typos and other small changes, either contact one of the developers, or if you think it's a significant enough error to cause problems for other users, please feel free to open an issue.

The development team will be happy to help and guide you with any of these points and work with you getting your PR submitted for approval. Create a PR with [WIP] in the title and ask for specific assistance within the issue, or contact the dev team on any of the channels below.

# Find Us

When you are starting to contribute to grin, we really would appreciate if you come by the gitter chat channels.

In case of problems with trying out grin, before starting to contribute, there's the [grincoin#support](https://keybase.io/team/grincoin) on Keybase. Write there about what you've done, what you want to do, and maybe paste logs through a text paste webservice.

* Please [join the grincoin#general  on Keybase](https://keybase.io/team/grincoin) to get a feeling for the community.
* And see the developers chat channel [grincoin#dev  on Keybase](https://keybase.io/team/grincoin) if you have questions about source code files.
  If you explain what you're looking at and what you want to do, we'll try to help you along the way.
* See `docs/*.md` and the folder structure explanations, [the wiki](https://github.com/mimblewimble/docs/wiki) and the official [Grin documentation](https://docs.grin.mw/).
* Further information and discussions are in the [Forum](https://forum.grin.mw), the [website](https://grin.mw), the [mailing list](https://lists.launchpad.net/mimblewimble/) and news channels like the [Reddit/grincoin](https://www.reddit.com/r/grincoin/) and a (mostly unfiltered!) Twitter bot that collects headlines, mailing list posts, and reddit posts related to Mimblewimble/Grin: [@grinmw](https://twitter.com/grinmw)

## Testing

Run all tests with `cargo test --all` and please remember to test locally before creating a PR on github.

### Check Travis output

After creating a PR on github, the code will be tested automatically by Travis CI, and from the results you'll get a red or green light. The test can take a while, and you'll have a "yellow traffic light" on your PR until Travis CI is done testing.

### Building quality

The most important thing you can do alongside - or even before - changing code, is adding tests for how grin should and should not work. See the various `tests` folders and derive test that are already there in grin.

After that, if you want to raise code quality another level, you can use `cargo check`, `cargo cov test` and `cargo tarpaulin`. Install them with `cargo install cargo-check cargo-cov; RUSTFLAGS="--cfg procmacro2_semver_exempt" cargo install cargo-tarpaulin`. Run with `cargo cov test` and `cargo tarpaulin`. The quality check tools are often integrated with `rustc` and as a side-effect only activated when some code is compiled. Because of this, if you want a complete check you'll need to `cargo clean` first.

We have some details on [code coverage and historical numbers on the wiki](https://github.com/mimblewimble/docs/wiki/Code-coverage-and-metrics).

# Pull-Request Title Prefix

**Note**: *[draft part! to be reviewed and discussed]*

Please consider putting one of the following prefixes in the title of your pull-request:
- **feat**:     A new feature
- **fix**:      A bug fix
- **docs**:     Documentation only changes
- **style**:    Formatting, missing semi-colons, white-space, etc
- **refactor**: A code change that neither fixes a bug nor adds a feature
- **perf**:     A code change that improves performance
- **test**:     Adding missing tests
- **chore**:    Maintain. Changes to the build process or auxiliary tools/libraries/documentation

For example: `fix: a panick on xxx when grin exiting`. Please don't worry if you can't find a suitable prefix, this's just optional, not mandatory.

# Grin Style Guide

This project uses `rustfmt` to maintain consistent formatting. We've made sure that rustfmt runs **automatically**, but you must install rustfmt manually first.

## Install rustfmt

**Note**: To work with grin you must use `rustup`. Linux package managers typically carry a too old rust version.
See [build docs](doc/build.md) for more info.

First ensure you have a new enough rustfmt:
```
rustup update
rustup component add rustfmt
rustfmt --version
```

and verify you did get version `rustfmt 1.0.0-stable (43206f4 2018-11-30)` or newer.

Then run `cargo build` to activate a git `pre-commit` hook that automates the rustfmt usage so you don't have to worry much about it. Read on for how to make this work in your advantage.

## Creating git commits

When you make a commit, rustfmt will be run and we also **automatically** reformat the .rs files that your commit is touching.

Please separate the rustfmt changes into one (or several) separate commits. This is best practice.

Easiest is to make your commit as normal and if rustfmt stops you, just redo the commit again, since it should always be allowed on the second try. Then add all rustfmt:ed files (`git add -u`) and commit them (`git commit -m "rustfmt"`).

You do have to remember to do this manually.

**Note**: The pre-commit hook will not prevent commits if style issues are present, instead it will indicate any files that need formatting, and it will automatically run `rustfmt` on your changed files, each time when you try to do `git commit`.

### Manually configuring git hooks

If you are developing new or changed git hooks, or are curious, you can config hooks manually like this `git config core.hooksPath ./.hooks` and to verify the effect do `git config --list | grep hook` and expect the output to be `core.hookspath=./.hooks`

### Running rustfmt manually

Not recommended, but you can run rustfmt on a file like this: `rustfmt client.rs`

**Notes**:
1. *Please keep rustfmt corrections in a separate commit. This is best practice and makes reviewing and merging your contribution work better.*

2. *If unsure about code formatting, it's just fine if you ignore and discard any rustfmt changes. It's only a nice-to-have. Your contribution and code changes is the priority here. Hope you're happy to contribute on this open source project!*

3. Please don't ~~`cargo +nightly fmt`~~ because all grin developers are using stable rustfmt. Also please don't rustfmt files that your code changes does not touch to avoid causing merge conflicts.

## Thanks for any contribution

Even one word correction are welcome! Our objective is to encourage you to get interested in Grin and contribute in any way possible. Thanks for any help!
