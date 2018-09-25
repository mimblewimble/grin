# Contributing

Find an area you can help with and do it. Open source is about collaboration and open participation. Try to make your code look like what already exists and submit a pull request.

The [list of issues](https://github.com/mimblewimble/grin/issues) is a good place to start, especially the ones tagged as "help wanted" (but don't let that stop you from looking at others). If you're looking for additional ideas, the code includes `TODO` comments for minor to major improvements. Grep is your friend.

Additional tests are rewarded with an immense amount of positive karma.

More documentation or updates/fixes to existing documentation are also very welcome. However, if submitting a PR(Pull-Request) consisting of documentation changes only, please try to ensure that the change is significantly more substantial than one or two lines. For example, working through an install document and making changes and updates throughout as you find issues is worth a PR. For typos and other small changes, either contact one of the developers, or if you think it's a significant enough error to cause problems for other users, please feel free to open an issue.

# Find Us

If any help is needed during your effort to contribute on this project, please don't hesitate to contact us:
* Chat: [Gitter](https://gitter.im/grin_community/Lobby).
* [Forum](https://www.grin-forum.org/)
* [Website](https://grin-tech.org)
* Mailing list: join the [~MimbleWimble team](https://launchpad.net/~mimblewimble) and subscribe on Launchpad.
* News: [Twitter](https://twitter.com/grinmw). Twitter bot that scrapes headlines, mailing list, and reddit posts related to MimbleWinble/Grin.

# Grin Style Guide

Grin uses `rustfmt` to maintain consistent formatting, and we're using the git commit hook as explained below.

## Install rustfmt

You should use rustup. See [build docs](doc/build.md) for more info.

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


