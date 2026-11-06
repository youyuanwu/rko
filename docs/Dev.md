# Update linux

Use a stable Linux release tag (run from the repository root):
```sh
TAG="v7.3.1"
git submodule sync -- linux
git -C linux fetch --depth 1 origin tag "$TAG"
git -C linux checkout --detach "$TAG"
```