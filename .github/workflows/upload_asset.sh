#!/bin/bash

# Usage helper.
usage() {
    echo "Usage: upload_asset.sh <FILE> <TOKEN>"
    echo "       upload_asset.sh --ensure-release <TOKEN> [<BODY_JSON>]"
}

ensure_release_only=false

while [ $# -gt 0 ]; do
    case "$1" in
        --ensure-release)
            ensure_release_only=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            break
            ;;
    esac
done

if [ "$ensure_release_only" = true ] && [ $# -lt 1 ]; then
    usage
    exit 1
fi

if [ "$ensure_release_only" = false ] && [ $# -lt 2 ]; then
    usage
    exit 1
fi

repo="${GITHUB_REPOSITORY:-alacritty/alacritty}"

if [ "$ensure_release_only" = true ]; then
    file_path=""
    bearer=$1
    release_body=${2-}
else
    file_path=$1
    bearer=$2
    release_body=""
fi

release_body=${release_body:-}
file_path=${file_path:-}

if [ "$ensure_release_only" = true ]; then
    echo "Ensuring release exists for $repo."
else
    echo "Starting asset upload from $file_path to $repo."
fi

# Get the release for this tag.
tag="$(git describe --tags --abbrev=0)"

# Make sure the git tag could be determined.
if [ -z "$tag" ]; then
    printf "\e[31mError: Unable to find git tag\e[0m\n"
    exit 1
fi

echo "Git tag: $tag"

# Get the upload URL for the current tag.
#
# Since this might be a draft release, we can't just use the /releases/tags/:tag
# endpoint which only shows published releases.

echo "Checking for existing release..."
upload_url=$(\
    curl \
        --http1.1 \
        -H "Authorization: Bearer $bearer" \
        "https://api.github.com/repos/$repo/releases" \
        2> /dev/null \
    | grep -E "(upload_url|tag_name)" \
    | paste - - \
    | grep -e "tag_name\": \"$tag\"" \
    | head -n 1 \
    | sed 's/.*\(https.*assets\).*/\1/' \
)

# Create a new release if we didn't find one for this tag.
if [ -z "$upload_url" ]; then
    echo "No release found."
    echo "Creating new release..."

    if [ -n "$release_body" ]; then
        release_payload="{\"tag_name\":\"$tag\",\"draft\":true,\"body\":$release_body}"
    else
        release_payload="{\"tag_name\":\"$tag\",\"draft\":true}"
    fi

    # Create new release.
    response=$(
        curl -f \
            --http1.1 \
            -X POST \
            -H "Authorization: Bearer $bearer" \
            -H "Content-Type: application/json" \
            -d "$release_payload" \
            "https://api.github.com/repos/$repo/releases" \
            2> /dev/null\
    )

    # Abort if the release could not be created.
    if [ $? -ne 0 ]; then
        printf "\e[31mError: Unable to create new release.\e[0m\n"
        exit 1;
    fi

    # Extract upload URL from new release.
    upload_url=$(\
        echo "$response" \
        | grep "upload_url" \
        | sed 's/.*: "\(.*\){.*/\1/' \
    )
fi

# Propagate error if no URL for asset upload could be found.
if [ -z "$upload_url" ]; then
    printf "\e[31mError: Unable to find release upload url.\e[0m\n"
    exit 2
fi

if [ "$ensure_release_only" = true ]; then
    echo "Release is available."
    exit 0
fi

# Upload the file to the tag's release.
file_name=${file_path##*/}
echo "Uploading asset $file_name to $upload_url..."
curl -f \
    --http1.1 \
    -X POST \
    -H "Authorization: Bearer $bearer" \
    -H "Content-Type: application/octet-stream" \
    --data-binary @"$file_path" \
    "$upload_url?name=$file_name" \
    &> /dev/null \
|| { \
    printf "\e[31mError: Unable to upload asset.\e[0m\n" \
    && exit 3; \
}

printf "\e[32mSuccess\e[0m\n"
