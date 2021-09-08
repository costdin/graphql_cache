tar -czf graphql_cache.tar.gz src Cargo.toml
scp build.sh bitnami@192.168.1.186:~/
scp graphql_cache.tar.gz bitnami@192.168.1.186:~/

ssh bitnami@192.168.1.186 "chmod a+x build.sh;./build.sh"

if (!(Test-Path "rasp-target"))
{
    mkdir rasp-target
}
else
{
    write-host "mkdir rasp-target already exists"
}

$BRANCH=(git rev-parse --abbrev-ref HEAD)
$FILE_NAME="graphql_cache_" + $BRANCH
$PATH="rasp-target/graphql_cache_" + $BRANCH

scp bitnami@192.168.1.186:~/graphql_cache/target/arm-unknown-linux-musleabi/release/graphql_cache $PATH
ssh pi@192.168.1.50 "killall ${FILE_NAME}; killall ./${FILE_NAME}"
scp $PATH pi@192.168.1.50:~/$FILE_NAME
ssh pi@192.168.1.50 "chmod a+x ${FILE_NAME} && ./${FILE_NAME}"