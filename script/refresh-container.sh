#!/bin/bash

# Deletes the container, build it and starts it again passing parameters
CONTAINER=$1

if [ -z "${CONTAINER}" ]; then
    echo "You should specify a container"
    exit 1
fi

docker-compose stop  $1
docker-compose rm -f $1
docker-compose build $1
docker-compose up $1 "${@:2}"
