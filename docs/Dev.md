# Update linux

Use linux tag
```sh
TAG="v7.0-rc1"
git fetch --depth 1 origin tag $TAG
git checkout $TAG
```