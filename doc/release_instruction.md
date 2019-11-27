# Grin - Instruction of Release

**Note**: *[totally draft doc! to be reviewed and discussed]*

## Version Number Rule

In Grin, we're using [Semantic Versioning 2.0.0](https://semver.org). For a short description of the rule:

A version number include MAJOR.MINOR.PATCH, and increment the:

1. MAJOR version when you make incompatible API changes,
1. MINOR version when you add functionality in a backwards-compatible manner, and
1. PATCH version when you make backwards-compatible bug fixes.

And **additional labels for pre-release** and **build metadata** are available as extensions to the MAJOR.MINOR.PATCH format.

The examples of the release version of Grin:

- 0.3.0
- 0.3.1
- 1.5.90

The examples of **label of pre-release**:

- 1.0.0-alpha.1
- 1.0.0-beta.2
- 1.0.0-test.5

In Grin, **build metadata** is used as the build number which comes from the Travis-CI job ID of building jobs, it's an unique ID for each building. **Note**: for the moment, this metadata is only used in the name of the released binary, and it's auto generated, no need to set it manually.

Here is an example of the build metadata of Grin release version:

- 0.3.1-430839304

And as the end of this section, here's an example of the whole encoded version string:

- 0.3.1-pre.1-430839316

## Release Files (Binaries)

So far, Grin support both Mac and Linux, and 64bits only. There're 2 binaries in one version release, plus md5 checksum for each binary.

For example:

- grin-0.3.1-pre.1-430839316-oxs.tgz
- grin-0.3.1-pre.1-430839316-oxs.tgz-md5sum.txt
- grin-0.3.1-pre.1-430839318-linux-amd64.tgz
- grin-0.3.1-pre.1-430839318-linux-amd64.tgz-md5sum.txt

## Change Log of Release

Currently, Grin release is using automatic change log generating, thanks to [github-changelog-generator](https://github.com/github-changelog-generator/github-changelog-generator). These change logs are fully automated generated, and normally there's no human editing on these change logs, but in case of any exception display on release page, we could do a little manual fixing on that.

The github changelog generator heavily rely on the Github project **issues*, **pull requests**, so for achieving a good change log on each release, we have to follow some rules on the daily management of github **issues** and **pull request**.

And this changelog generator will collect all the github closed issues and merged pull-requests, **since last tag**.

Also it will give a link to fully compare this release with the previous tag in github.

## Rules of Github Issue and Pull-Request Management

As said above, we have to define some basic rules for github issue and pull-request, but don't worry, it's quite simple!

### Rules of Issue

The most important rule is to Distinguish Issues by **Labels**:

- Label bug issues as **bug**
- Label enhancement issues as **enhancement**

And all closed issues w/o label will be put into group "closed issues with no labels", which is definitely not so good.

As opposite, all closed issues with the following labels will be automatically excluded from the change log:

- **question**
- **duplicate**
- **invalid**
- **wontfix**

For more details, please run `github_changelog_generator --help`, or check its github repo: [github-changelog-generator](https://github.com/github-changelog-generator/github-changelog-generator)

### Rules of Pull-Request

And regarding to merged **pull-requests**, it will be put into group "all merged pull-requests". And it's strongly recommended that each pull-request give a commit message with prefix:

- **feat**:     A new feature
- **fix**:      A bug fix
- **docs**:     Documentation only changes
- **style**:    Formatting, missing semi-colons, white-space, etc
- **refactor**: A code change that neither fixes a bug nor adds a feature
- **perf**:     A code change that improves performance
- **test**:     Adding missing tests
- **chore**:    Maintain. Changes to the build process or auxiliary tools/libraries/documentation

These prefix are part of [Angular.js commit message conventions](https://docs.google.com/document/d/1QrDFcIiPjSLDn3EL15IJygNPiHORgU1_OOAqWjiDU5Y/edit?pref=2&pli=1#heading=h.uyo6cb12dt6w), and you can take a look at the practice in [Angular.js github project](https://github.com/angular/angular.js/commits/master).

There's no need to fully follow Angular.js commit message conventions. We just need use these prefix in each pull-request's commit message, that's all. Perhaps a simple **pull-request template** can be set as default in Github.

## Release Branches

Each time when Grin release a new version, there's definitely a tag there with same version number. But it's not mandatory to have a **branch** with this version release.

We define the following rules for the **release branch**:

1. Only **MAJOR.MINOR** version could have a release branch, but also NOT mandatory.
1. Only when a fix is needed for a **MAJOR.MINOR** version, we create a **release branch** on the name of **MAJOR.MINOR** from the tag of **MAJOR.MINOR.0**, and only one branch for all the fixes on this **MAJOR.MINOR** version.
1. New version based on this **release branch** must have same **MAJOR.MINOR** version number, only **.PATCH** can be changed.

## Release Instruction

### 1. TAG

To trigger a version release, the ONLY thing which need to do manually is to make a tag. As the **owner** of a github repo, he/she just need to run 2 commands locally:

```bash
git tag 0.3.1-pre1 -m "0.3.1 pre release 1"
git push origin 0.3.1-pre1
```

Done.
Remember to replace `0.3.1-pre1` as the real version, and warmly remind the [[Version Number Rule]]. For official release, just use `0.3.1` or something like that.

If you're NOT the owner of the github repo, but at least you have to be a committer which has the right to do a release, the following steps are needed to trigger a version release:

1. Go to release page of the repo, click **Draft a new release**, remember to check the branch is what you're working on! set the **Tag version** to the release number (for    example: `0.3.1-pre1`), and set anything in **Release title** and **description**, then click **Publish release**. Don't worry the title and description parts because we need delete it in next step.
1. Because github **release** will be auto-created by our `release-jobs` building script, we MUST delete the **release** which we just created in previous step! (Unfortunately, there's no way to only create **tag** by web.)

Even normally Travis-CI need tens of minutes to complete building, I suggest you complete step 2 quickly, otherwise the `release-jobs` script will fail on error "release already exist".

### 2. Travis-CI Building

For building in both mac and Linux, we enabled the CI on both platforms, since we start doing this automatic binaries release.

The release building is just one of the **TEST_DIRS**, named as `none`. So each time when version release building is triggered, all those exising Travis-CI tests are also triggered, this design is for the CI test purpose and we hope this new added auto release script has no any impact on these existing CI tests.

So, the point is: the release building job is that one tagged with `TEST_DIR=none`.

Note: `release-jobs` script will only be executed on `deploy` stage, and according to Travis-CI, it will be skipped for any **pull-request** trigger, and since we set `tag: true` it will be only executed when triggered by a tag.

### 3. Check the Release Page

The last step is to check the Github release page, after Travis-CI complete the whole building, normally it need half an hour or one hour.
