#!/bin/bash

dockerd -G 10001 &
su kittengrid -c "/kittengrid-agent --bind-address 0.0.0.0 --bind-port 65534"
