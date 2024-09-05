# Load env vars from an .env.local file
set -x

# Load .env.local file if it exists exporting all variables
if [ -f .env.local ]; then
  export $(cat .env.local | xargs)
fi

# execute the command passed to the script
/app/target/debug/kittengrid-agent

