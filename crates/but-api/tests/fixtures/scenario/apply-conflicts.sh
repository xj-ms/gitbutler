#!/usr/bin/env bash

set -eu -o pipefail

git init
git config user.name GitButler
git config user.email gitbutler@example.com
export GIT_AUTHOR_DATE="2026-01-01T00:00:00+0000"
export GIT_COMMITTER_DATE="$GIT_AUTHOR_DATE"

printf 'base\n' > a.txt
printf 'base\n' > b.txt
printf 'base\n' > c.txt
git add .
git commit -m base
git update-ref refs/remotes/origin/main HEAD
git config remote.origin.url ./unused
git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'

git checkout -b feature
printf 'feature\n' > a.txt
printf 'feature\n' > b.txt
git commit -am feature

git checkout -b incoming main
printf 'incoming\n' > a.txt
printf 'incoming\n' > b.txt
printf 'incoming\n' > c.txt
git commit -am incoming

git checkout -b other main
printf 'other\n' > c.txt
git commit -am other

git checkout -b clean main
printf 'clean\n' > clean.txt
git add clean.txt
git commit -m clean

git checkout main
