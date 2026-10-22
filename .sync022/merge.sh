#!/bin/sh
# Scratch helper for task 022: 3-way merge of ui-egui files (base v0.8.0, ours HEAD, theirs v0.9.0).
set -u
out=.sync022
for f in "$@"; do
  name=$(echo "$f" | tr '/' '_')
  git show "v0.8.0:$f" > "$out/$name.base" 2>/dev/null || : > "$out/$name.base"
  git show "v0.9.0:$f" > "$out/$name.theirs" 2>/dev/null || : > "$out/$name.theirs"
  cp "$f" "$out/$name.ours"
  git merge-file -p -L ours -L base -L theirs "$out/$name.ours" "$out/$name.base" "$out/$name.theirs" > "$out/$name.merged"
  echo "$f conflicts=$?"
done
