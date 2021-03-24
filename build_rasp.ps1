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

scp bitnami@192.168.1.186:~/graphql_cache/target/arm-unknown-linux-musleabi/release/graphql_cache rasp-target/
scp rasp-target/graphql_cache pi@192.168.1.50:~/graphql_cache
ssh pi@192.168.1.50 "chmod a+x graphql_cache"