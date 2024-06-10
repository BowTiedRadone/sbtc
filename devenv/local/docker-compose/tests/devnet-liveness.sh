#!/bin/bash


echo " -----------------------------------------------"
echo "| => (1) 🔬 TEST: [CHECK BITCOIN NODE IS LIVE]  |"
echo " -----------------------------------------------"

CHECK_BTC_LIVENESS_RESULT=$(curl -s -u "devnet:devnet" --data-binary '{"jsonrpc": "1.0", "id": "curltest", "method": "getblockcount", "params": []}' -H 'content-type: text/plain;' "http://localhost:18443/" | jq)

echo "\nGET BLOCKCOUNT RPC:"
echo $CHECK_BTC_LIVENESS_RESULT | jq

BTC_LIVENESS_SUCCESS=$(echo $CHECK_BTC_LIVENESS_RESULT | jq -r '.error == null')
BTC_LIVENESS_SUCCESS_FRMT=$([ "$BTC_LIVENESS_SUCCESS" == "true" ] && echo "\033[1;32m$BTC_LIVENESS_SUCCESS\033[0m ✅" || echo "\033[1;31m$BTC_LIVENESS_SUCCESS\033[0m❌") 


echo "\033[1mBTC_LIVENESS_SUCCESS\033[0m: $BTC_LIVENESS_SUCCESS_FRMT"
echo "\n"




echo " ------------------------------------------------------"
echo "| => (2) 🔬 TEST: [CHECK IF BTC MINER IS ABLE TO MINE] |"
echo " ------------------------------------------------------"


echo "\nMINE 1 BLOCK RPC:"
MINER_ADDRESS="mqVnk6NPRdhntvfm4hh9vvjiRkFDUuSYsH"
CHECK_IF_BTC_MINEABLE_RESULT=$(curl -s -u "devnet:devnet" --data-binary '{"jsonrpc": "1.0", "id": "curltest", "method": "generatetoaddress", "params": [1, "'$MINER_ADDRESS'"]}' -H 'content-type: text/plain;' "http://localhost:18443/" | jq)

echo $CHECK_IF_BTC_MINEABLE_RESULT | jq

BTC_MINEABLE_SUCCESS=$(echo $CHECK_IF_BTC_MINEABLE_RESULT | jq -r '.error == null')
BTC_MINEABLE_SUCCESS_FRMT=$([ "$BTC_MINEABLE_SUCCESS" == "true" ] && echo "\033[1;32m$BTC_MINEABLE_SUCCESS\033[0m ✅" || echo "\033[1;31m$BTC_MINEABLE_SUCCESS\033[0m❌") 


echo "\033[1mBTC_MINEABLE_SUCCESS\033[0m: $BTC_MINEABLE_SUCCESS_FRMT"
echo "\n"








echo " -----------------------------------------------"
echo "| => (3) 🔬 TEST: [CHECK IF POSTGRES IS READY]  |"
echo " -----------------------------------------------"


# PG_DOCKER_LOGS=$(docker logs postgres 2>/dev/null)

# PG_READY_SUCCESS=false
# PG_READY_SUCCESS_FRMT=$(echo "\033[1;31m$PG_READY_SUCCESS\033[0m❌")
# if [[ $PG_DOCKER_LOGS == *"ready to accept connections"* ]]; then
#     PG_READY_SUCCESS=true
#     PG_READY_SUCCESS_FRMT=$(echo "\033[1;32m$PG_READY_SUCCESS\033[0m ✅")
# fi


## DO NOT UNCOMMENT (USE THIS IF YOU WANT TO BE ABSOLUTELY SURE THAT POSTGRES WORKS)
PG_READY_SUCCESS=false
PG_READY_SUCCESS_FRMT=$(echo "\033[1;31m$PG_READY_SUCCESS\033[0m❌")
if (pg_isready -h localhost -p 5432 -U postgres); then
    PG_READY_SUCCESS=true
    PG_READY_SUCCESS_FRMT=$(echo "\033[1;32m$PG_READY_SUCCESS\033[0m ✅")
fi


echo "\033[1mPG_READY_SUCCESS\033[0m: $PG_READY_SUCCESS_FRMT"
echo "\n"







echo " -----------------------------------------------"
echo "| => (4) 🔬 TEST: [CHECK IF MARIADB IS READY]  |"
echo " -----------------------------------------------"


MARIADB_DOCKER_LOGS=$(docker logs mariadb 2>/dev/null)

MARIADB_READY_SUCCESS=false
MARIADB_READY_SUCCESS_FRMT=$(echo "\033[1;31m$MARIADB_READY_SUCCESS\033[0m❌")
if [[ $MARIADB_DOCKER_LOGS == *"ready for connections"* || $MARIADB_DOCKER_LOGS == *"Ready for start up"* ]]; then
    MARIADB_READY_SUCCESS=true
    echo "MariaDB || Ready for start up"
    MARIADB_READY_SUCCESS_FRMT=$(echo "\033[1;32m$MARIADB_READY_SUCCESS\033[0m ✅")
fi



echo "\033[1mMARIADB_READY_SUCCESS\033[0m: $MARIADB_READY_SUCCESS_FRMT"
echo "\n"









echo " -------------------------------------------------------"
echo "| => (5) 🔬 TEST: [CHECK IF NAKAMOTO SIGNER 1 IS READY]  |"
echo " -------------------------------------------------------"


NAKAMOTO_SIGNER_1_DOCKER_LOGS=$(docker logs nakamoto-signer-1 2>/dev/null)

NAKAMOTO_SIGNER_1_READY_SUCCESS=false
NAKAMOTO_SIGNER_1_READY_SUCCESS_FRMT=$(echo "\033[1;31m$NAKAMOTO_SIGNER_1_READY_SUCCESS\033[0m❌")
if [[ $NAKAMOTO_SIGNER_1_DOCKER_LOGS == *"Signer spawned successfully"* ]]; then
    NAKAMOTO_SIGNER_1_READY_SUCCESS=true
    echo "Nakamoto Signer || Signer spawned successfully"
    NAKAMOTO_SIGNER_1_READY_SUCCESS_FRMT=$(echo "\033[1;32m$NAKAMOTO_SIGNER_1_READY_SUCCESS\033[0m ✅")
fi


echo "\033[1mNAKAMOTO_SIGNER_1_READY_SUCCESS\033[0m: $NAKAMOTO_SIGNER_1_READY_SUCCESS_FRMT"
echo "\n"





echo " -------------------------------------------------------"
echo "| => (6) 🔬 TEST: [CHECK IF NAKAMOTO SIGNER 2 IS READY]  |"
echo " -------------------------------------------------------"


NAKAMOTO_SIGNER_2_DOCKER_LOGS=$(docker logs nakamoto-signer-2 2>/dev/null)

NAKAMOTO_SIGNER_2_READY_SUCCESS=false
NAKAMOTO_SIGNER_2_READY_SUCCESS_FRMT=$(echo "\033[1;31m$NAKAMOTO_SIGNER_2_READY_SUCCESS\033[0m❌")
if [[ $NAKAMOTO_SIGNER_2_DOCKER_LOGS == *"Signer spawned successfully"* ]]; then
    NAKAMOTO_SIGNER_2_READY_SUCCESS=true
    echo "Nakamoto Signer || Signer spawned successfully"
    NAKAMOTO_SIGNER_2_READY_SUCCESS_FRMT=$(echo "\033[1;32m$NAKAMOTO_SIGNER_2_READY_SUCCESS\033[0m ✅")
fi


echo "\033[1mNAKAMOTO_SIGNER_2_READY_SUCCESS\033[0m: $NAKAMOTO_SIGNER_2_READY_SUCCESS_FRMT"
echo "\n"





echo " -------------------------------------------------------"
echo "| => (7) 🔬 TEST: [CHECK IF NAKAMOTO SIGNER 2 IS READY]  |"
echo " -------------------------------------------------------"


NAKAMOTO_SIGNER_3_DOCKER_LOGS=$(docker logs nakamoto-signer-3 2>/dev/null)

NAKAMOTO_SIGNER_3_READY_SUCCESS=false
NAKAMOTO_SIGNER_3_READY_SUCCESS_FRMT=$(echo "\033[1;31m$NAKAMOTO_SIGNER_3_READY_SUCCESS\033[0m❌")
if [[ $NAKAMOTO_SIGNER_3_DOCKER_LOGS == *"Signer spawned successfully"* ]]; then
    NAKAMOTO_SIGNER_3_READY_SUCCESS=true
    echo "Nakamoto Signer || Signer spawned successfully"
    NAKAMOTO_SIGNER_3_READY_SUCCESS_FRMT=$(echo "\033[1;32m$NAKAMOTO_SIGNER_3_READY_SUCCESS\033[0m ✅")
fi


echo "\033[1mNAKAMOTO_SIGNER_3_READY_SUCCESS\033[0m: $NAKAMOTO_SIGNER_3_READY_SUCCESS_FRMT"
echo "\n"






echo " --------------------------------------------------"
echo "| => (8) 🔬 TEST: [CHECK IF STACKS NODE IS READY]  |"
echo " --------------------------------------------------"


GET_STACKS_NODE_INFO_STATUS_CODE=$(curl --write-out %{http_code} --silent --output /dev/null "http://localhost:20443/v2/info")


echo "\nGET STACKS NODE STATUS: $GET_STACKS_NODE_INFO_STATUS_CODE"

STX_LIVENESS_SUCCESS=false 
STACKS_LIVENESS_SUCCESS_FRMT=$(echo "\033[1;31m$STX_LIVENESS_SUCCESS\033[0m❌")

if [[ $GET_STACKS_NODE_INFO_STATUS_CODE == "200" ]]; then
    STX_LIVENESS_SUCCESS=true
    STACKS_LIVENESS_SUCCESS_FRMT=$(echo "\033[1;32m$STX_LIVENESS_SUCCESS\033[0m ✅")
fi


echo "\033[1mSTACKS_LIVENESS_SUCCESS\033[0m: $STACKS_LIVENESS_SUCCESS_FRMT"
echo "\n"








echo " ---------------------------------------------------------------"
echo "| => (9) 🔬 TEST: [CHECK IF STX NODE IS SYNCED WITH BTC UTXOs]  |"
echo " ---------------------------------------------------------------"


## (RPC APPROACH)
GET_STACKS_NODE_INFO=$(curl -s "http://localhost:20443/v2/info")

echo "\nGET STACKS NODE INFO:"
echo $GET_STACKS_NODE_INFO | jq 'del(.stackerdbs)'
echo "\t\t.\n\t\t.\n  \033[1;32m<<\033[0m \033[1;35mLong Output Supressed\033[0m \033[1;32m>>\033[0m \n\t\t.\n\t\t."

STX_SYNC_WITH_BTC_UTXO_SUCCESS=$(echo $GET_STACKS_NODE_INFO | jq -r '.stacks_tip_height != 0')
STX_SYNC_WITH_BTC_UTXO_SUCCESS_FRMT=$([ "$STX_SYNC_WITH_BTC_UTXO_SUCCESS" == "true" ] && echo "\033[1;32m$STX_SYNC_WITH_BTC_UTXO_SUCCESS\033[0m ✅" || echo "\033[1;31m$STX_SYNC_WITH_BTC_UTXO_SUCCESS\033[0m❌") 

echo "\033[1mSTX_SYNC_WITH_BTC_UTXO_SUCCESS\033[0m: $STX_SYNC_WITH_BTC_UTXO_SUCCESS_FRMT"
echo "\n"





echo " ---------------------------------------------------------------"
echo "| => (10) 🔬 TEST: [CHECK IF STACKS NODE IS RUNNING NAKAMOTO]    |"
echo " ---------------------------------------------------------------"


# Helper Function to check if a file is binary
is_binary_file() {
    local file_path="$1"
    # Use grep to search for non-printable characters
    if grep -q '[^[:print:][:space:]]' "$file_path"; then
        echo "true" # Non-printable characters found
    else
        echo "false" # No non-printable characters found
    fi
}


CHECK_IF_STACKS_TIP_HEIGHT_IS_SUFFICIENT=$(curl -s "http://localhost:20443/v2/info" | jq -r '.stacks_tip_height >= 130')
echo "CHECK_IF_STACKS_TIP_HEIGHT_IS_SUFFICIENT: $CHECK_IF_STACKS_TIP_HEIGHT_IS_SUFFICIENT"
IS_NAKAMOTO_RUNNING_SUCCESS_FRMT=$(echo "\033[1;33mWAITING\033[0m🟡")


# Ensure that we have a STACKS_TIP_HEIGHT
if [[ $CHECK_IF_STACKS_TIP_HEIGHT_IS_SUFFICIENT == true ]]; then

    GET_STACKS_NODE_NAKAMOTO_INFO=$(curl -s "http://localhost:20443/v3/tenures/info")

    echo "\nGET STACKS NAKAMOTO INFO:"
    echo $GET_STACKS_NODE_NAKAMOTO_INFO | jq

    # get parent_tenure_start_block_id
    PARENT_TENURE_START_BLOCK_ID=$(echo $GET_STACKS_NODE_NAKAMOTO_INFO | jq -r '.parent_tenure_start_block_id')

    # lookup if this block is found
    echo "\n+++++++++++++++++++++++\n\nLOOKUP NAKAMOTO BLOCK:\n"
    rm -rf nkblock.binary
    curl -s "http://localhost:20443/v3/blocks/$PARENT_TENURE_START_BLOCK_ID" -o nkblock.binary


    IS_NAKAMOTO_RUNNING_SUCCESS=false
    IS_NAKAMOTO_RUNNING_SUCCESS_FRMT=$(echo "\033[1;31m$IS_NAKAMOTO_RUNNING_SUCCESS\033[0m❌")


    if [[ $(is_binary_file ./nkblock.binary) == "true" ]]; then
        ## is a binary file
        xxd nkblock.binary
        IS_NAKAMOTO_RUNNING_SUCCESS=true
        IS_NAKAMOTO_RUNNING_SUCCESS_FRMT=$(echo "\033[1;32m$IS_NAKAMOTO_RUNNING_SUCCESS\033[0m ✅")
    fi

else
    echo "🟡  ⚠️ The 'stacks_tip_height' has not reached 130 yet. Skipping this test ..."
fi


echo "\033[1mIS_NAKAMOTO_RUNNING_SUCCESS\033[0m: $IS_NAKAMOTO_RUNNING_SUCCESS_FRMT"
echo "\n"






echo " ---------------------------------------------------------------"
echo "| => (11) 🔬 TEST: [CHECK STACKS API EVENT OBSERVER LIVENESS]  |"
echo " ---------------------------------------------------------------"


GET_STACKS_API_EVENT_OBSERVER_PING=$(curl -s "http://localhost:3700")


echo "\nGET STACKS API EVENT OBSERVER PING:"
echo $GET_STACKS_API_EVENT_OBSERVER_PING | jq


STACKS_API_EVENT_OBSERVER_LIVENESS_SUCCESS=$(echo $GET_STACKS_API_EVENT_OBSERVER_PING | jq -r '.status == "ready"')
STACKS_API_EVENT_OBSERVER_LIVENESS_SUCCESS_FRMT=$([ "$STACKS_API_EVENT_OBSERVER_LIVENESS_SUCCESS" == "true" ] && echo "\033[1;32m$STACKS_API_EVENT_OBSERVER_LIVENESS_SUCCESS\033[0m ✅" || echo "\033[1;31m$STACKS_API_EVENT_OBSERVER_LIVENESS_SUCCESS\033[0m❌") 


echo "\033[1mSTACKS_API_EVENT_OBSERVER_LIVENESS_SUCCESS\033[0m: $STACKS_API_EVENT_OBSERVER_LIVENESS_SUCCESS_FRMT"
echo "\n"




echo " ---------------------------------------------------------------"
echo "| => (12) 🔬 TEST: [CHECK STACKS PUBLIC API LIVENESS]  |"
echo " ---------------------------------------------------------------"


GET_STACKS_PUBLIC_API_PING=$(curl -s --write-out %{http_code} --silent --output /dev/null  "http://localhost:3999/extended/")


echo "\nGET STACKS PUBLIC API PING:"
echo $GET_STACKS_PUBLIC_API_PING | jq


STACKS_PUBLIC_API_LIVENESS_SUCCESS=false
STACKS_PUBLIC_API_LIVENESS_SUCCESS_FRMT=$(echo "\033[1;31m$STACKS_API_PUBLIC_LIVENESS_SUCCESS\033[0m❌")


if [[ $GET_STACKS_PUBLIC_API_PING == "200" ]]; then
    STACKS_PUBLIC_API_LIVENESS_SUCCESS=true
    STACKS_PUBLIC_API_LIVENESS_SUCCESS_FRMT=$(echo "\033[1;32m$STACKS_PUBLIC_API_LIVENESS_SUCCESS\033[0m ✅")
fi


echo "\033[1mSTACKS_PUBLIC_API_LIVENESS_SUCCESS\033[0m: $STACKS_PUBLIC_API_LIVENESS_SUCCESS_FRMT"
echo "\n"






echo " -----------------------------------------------------------------"
echo "| => (13) 🔬 TEST: [CHECK IF STACKS-API IS CONNECTED TO POSTGRES]  |"
echo " -----------------------------------------------------------------"


STACKS_API_DOCKER_LOGS=$(docker logs stacks-api 2>/dev/null)


STACKS_API_CONNECTED_TO_PG_SUCCESS=false
STACKS_API_CONNECTED_TO_PG_SUCCESS_FRMT=$(echo "\033[1;31m$STACKS_API_CONNECTED_TO_PG_SUCCESS\033[0m❌")
if [[ $STACKS_API_DOCKER_LOGS == *"PgNotifier connected"* ]]; then
    STACKS_API_CONNECTED_TO_PG_SUCCESS=true
    echo "Stacks-API || PgNotifier connected"
    STACKS_API_CONNECTED_TO_PG_SUCCESS_FRMT=$(echo "\033[1;32m$STACKS_API_CONNECTED_TO_PG_SUCCESS\033[0m ✅")
fi


echo "\033[1mSTACKS_API_CONNECTED_TO_PG_SUCCESS\033[0m: $STACKS_API_CONNECTED_TO_PG_SUCCESS_FRMT"
echo "\n"


echo "-----------------------------------------------------------------"
echo "|                        SUMMARY                                 |"
echo "-----------------------------------------------------------------"
echo "| \033[1mBTC_LIVENESS_SUCCESS\033[0m:                         | \t $BTC_LIVENESS_SUCCESS_FRMT  |"
echo "| \033[1mBTC_MINEABLE_SUCCESS\033[0m:                         | \t $BTC_MINEABLE_SUCCESS_FRMT  |"
echo "| \033[1mPG_READY_SUCCESS\033[0m:                             | \t $PG_READY_SUCCESS_FRMT  |"
echo "| \033[1mMARIADB_READY_SUCCESS\033[0m:                        | \t $MARIADB_READY_SUCCESS_FRMT  |"
echo "| \033[1mNAKAMOTO_SIGNER_1_READY_SUCCESS\033[0m:              | \t $NAKAMOTO_SIGNER_1_READY_SUCCESS_FRMT  |"
echo "| \033[1mNAKAMOTO_SIGNER_2_READY_SUCCESS\033[0m:              | \t $NAKAMOTO_SIGNER_2_READY_SUCCESS_FRMT  |"
echo "| \033[1mNAKAMOTO_SIGNER_3_READY_SUCCESS\033[0m:              | \t $NAKAMOTO_SIGNER_3_READY_SUCCESS_FRMT  |"
echo "| \033[1mSTACKS_LIVENESS_SUCCESS\033[0m:                      | \t $STACKS_LIVENESS_SUCCESS_FRMT  |"
echo "| \033[1mSTX_SYNC_WITH_BTC_UTXO_SUCCESS\033[0m:               | \t $STX_SYNC_WITH_BTC_UTXO_SUCCESS_FRMT  |"
echo "| \033[1mIS_NAKAMOTO_RUNNING_SUCCESS\033[0m:                  | \t $IS_NAKAMOTO_RUNNING_SUCCESS_FRMT  |"
echo "| \033[1mSTACKS_API_EVENT_OBSERVER_LIVENESS_SUCCESS\033[0m:   | \t $STACKS_API_EVENT_OBSERVER_LIVENESS_SUCCESS_FRMT |"
echo "| \033[1mSTACKS_PUBLIC_API_LIVENESS_SUCCESS\033[0m:           | \t $STACKS_PUBLIC_API_LIVENESS_SUCCESS_FRMT  |"
echo "| \033[1mSTACKS_API_CONNECTED_TO_PG_SUCCESS\033[0m:           | \t $STACKS_API_CONNECTED_TO_PG_SUCCESS_FRMT  |"
echo "-----------------------------------------------------------------"








if [[ $BTC_LIVENESS_SUCCESS == true \
    && $BTC_MINEABLE_SUCCESS == true \
    && $PG_READY_SUCCESS == true \
    && $MARIADB_READY_SUCCESS == true \
    && $NAKAMOTO_SIGNER_1_READY_SUCCESS == true \
    && $NAKAMOTO_SIGNER_2_READY_SUCCESS == true \
    && $NAKAMOTO_SIGNER_3_READY_SUCCESS == true \
    && $STACKS_LIVENESS_SUCCESS == true \
    && $STX_SYNC_WITH_BTC_UTXO_SUCCESS == true \
    && $IS_NAKAMOTO_RUNNING_SUCCESS == true \
    && $STACKS_API_EVENT_OBSERVER_LIVENESS_SUCCESS == true \
    && $STACKS_PUBLIC_API_LIVENESS_SUCCESS == true \
    && $STACKS_API_CONNECTED_TO_PG_SUCCESS == true ]]; then
    exit 0
fi

exit 1