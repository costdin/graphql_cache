docker build redis/ -t redis-as
docker run -d -p 6379:6379 pass-redis