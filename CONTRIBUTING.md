# Contributing

Find an area you can help with and do it. Open source is about collaboration and open participation. Try to make your code look like what already exists and submit a pull request.

The [list of issues](https://github.com/mimblewimble/grin/issues) is a good place to start, especially the ones tagged as "help wanted" (but don't let that stop you from looking at others). If you're looking for additional ideas, the code includes `TODO` comments for minor to major improvements. Use _Search in files_ in your code editor, or `grep "TODO" -r src/`.

Additional tests are rewarded with an immense amount of positive karma.

More documentation or updates/fixes to existing documentation are also very welcome. However, if submitting a PR(Pull-Request) consisting of documentation changes only, please try to ensure that the change is significantly more substantial than one or two lines. For example, working through an install document and making changes and updates throughout as you find issues is worth a PR. For typos and other small changes, either contact one of the developers, or if you think it's a significant enough error to cause problems for other users, please feel free to open an issue.

# Find Us

When you are starting to contribute to grin, we really would appreciate if you come by the gitter chat channels.

In case of problems with trying out grin, before starting to contribute, there's the [Support chat](https://gitter.im/grin_community/support). Write there about what you've done, what you want to do, and maybe paste logs through a text paste webservice.

* Please [join the grin Lobby](https://gitter.im/grin_community/Lobby) to get a feeling for the community.
* And [see the developers chat](https://gitter.im/grin_community/dev) if you have questions about source code files.
  If you explain what you're looking at and what you want to do, we'll try to help you along the way.
* Also see `docs/*.md` and the folder structure explanations, and [the wiki](https://github.com/mimblewimble/docs/wiki).
* Further information and discussions are in the [Forum](https://www.grin-forum.org/), the [website](https://grin-tech.org), the [mailing list](https://lists.launchpad.net/mimblewimble/) and news channels like the [@grincouncil](https://twitter.com/grincouncil) and a (mostly unfiltered!) Twitter bot that collects headlines, mailing list posts, and reddit posts related to MimbleWinble/Grin: [@grinmw](https://twitter.com/grinmw)

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

Grin uses `rustfmt` to maintain consistent formatting, and we're using the git commit hook as explained below.

## Install rustfmt

You should use `rustup`. See [build docs](doc/build.md) for more info.

```
rustup component add rustfmt-preview
rustup update
rustfmt --version
```

and verify you did get version `rustfmt 0.99.1-stable (da17b689 2018-08-04)` or newer.

## Automatic "pre-commit hook"

There is a basic git [pre-commit](.hooks/pre-commit) hook in the repo, and this pre-commit hook will be **automatically** configured in this project, once you run `cargo build` for the 1st time.
  
Or you can config it manually with the following command without building, and check it:
```
git config core.hooksPath ./.hooks
git config --list | grep hook
```
The output will be:
```
core.hookspath=./.hooks
```

**Note**: The pre-commit hook will not prevent commits if style issues are present, instead it will indicate any files that need formatting, and it will automatically run `rustfmt` on your changed files, each time when you try to do `git commit`.

## Running rustfmt manually

You can run rustfmt (i.e. rustfmt-preview) on one file or on all files.

For example:
```
rustfmt client.rs
```

**Notes**:
1. *Please add the rustfmt corrections as a separate commit at the end of your Pull Request to make the reviewers happy.*

2. *If you're still not sure about what should do on the format, please feel free to ignore it. Since `rustfmt` is just a tool to make your code having pretty formatting, your changed code is definitely more important than the format. Hope you're happy to contribute on this open source project!*

3. And anyway please don't use ~~`cargo +nightly fmt`~~ if at all possible.

## Thanks for any contribution

Even one word correction are welcome! Our objective is to encourage you to get interested in Grin and contribute in any way possible. Thanks for any help!


