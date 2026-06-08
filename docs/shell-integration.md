# Show today's AI spend in your shell prompt

Put your running AI coding cost right in your prompt, updated after every command:

```
~/projects/api  main ⇡2  $12.40
❯
```

That `$12.40` is **today's total cost across every AI CLI toktrack tracks** (Claude Code, Codex, Gemini, …), read straight from toktrack's local cache. No daemon, no extra process — just a tiny call on each prompt.

## Prerequisites

- An **installed** `toktrack` binary on your `PATH` (`brew install`, `cargo install`, or a GitHub release binary). Don't use `npx toktrack` here — the npx startup overhead is too slow for a prompt hook.
- [`jq`](https://jqlang.github.io/jq/) for extracting the number.

## The helper

Drop this function into your shell config. It prints `$<amount>` for **today only** (so an idle day shows `$0.00`, never a stale past day), reads **local data only** (`--local-only`, so it never triggers an SSH/remote sync on every prompt), and stays silent if `toktrack` isn't installed.

```sh
toktrack_prompt_cost() {
  command -v toktrack >/dev/null 2>&1 || return
  local today cost
  today=$(date +%F)
  cost=$(toktrack daily --json --local-only 2>/dev/null \
    | jq -r --arg t "$today" '(.[0] // {}) | if .date == $t then (.total_cost_usd // 0) else 0 end' 2>/dev/null)
  [ -n "$cost" ] && printf '$%.2f' "$cost"
}
```

> Why `.[0]`? `toktrack daily --json` is sorted newest-first, so the first element is the most recent day with activity. The `if .date == $t` guard makes sure we only show a number when that day **is** today.

## zsh

```zsh
# ~/.zshrc
setopt PROMPT_SUBST

# (paste toktrack_prompt_cost from above)

# Recompute once per prompt and cache it, so prompt redraws stay free.
precmd_functions+=(_toktrack_precmd)
_toktrack_precmd() { TOKTRACK_COST=$(toktrack_prompt_cost) }

# Show it on the right side of the prompt.
RPROMPT='%F{yellow}${TOKTRACK_COST}%f'
```

## bash

```bash
# ~/.bashrc

# (paste toktrack_prompt_cost from above)

_toktrack_prompt() { TOKTRACK_COST=$(toktrack_prompt_cost); }
PROMPT_COMMAND="_toktrack_prompt${PROMPT_COMMAND:+; $PROMPT_COMMAND}"

# Append to your existing PS1.
PS1="${PS1}\[\e[33m\]\${TOKTRACK_COST}\[\e[0m\] "
```

## fish

```fish
# ~/.config/fish/functions/toktrack_prompt_cost.fish
function toktrack_prompt_cost
    command -v toktrack >/dev/null 2>&1; or return
    set -l today (date +%F)
    set -l cost (toktrack daily --json --local-only 2>/dev/null \
        | jq -r --arg t "$today" '(.[0] // {}) | if .date == $t then (.total_cost_usd // 0) else 0 end' 2>/dev/null)
    test -n "$cost"; and printf '$%.2f' "$cost"
end
```

```fish
# add to your fish_prompt (functions/fish_prompt.fish)
echo -n (set_color yellow)(toktrack_prompt_cost)(set_color normal)' '
```

## Performance

`--local-only` keeps every call on toktrack's **warm cache path** — only the current day is recomputed, past days are read straight from `~/.toktrack/cache/`, so a prompt call is on the order of tens of milliseconds even with months of history. The first call after the cache is cleared does a full parse and is slower; it self-heals on the next run.

Want **zero** prompt latency? Refresh in the background and read a cached value:

```sh
# Refresh the cached cost at most once every 30s, in the background.
toktrack_prompt_cost_cached() {
  local f=${TMPDIR:-/tmp}/toktrack-cost age now
  now=$(date +%s)
  age=$(( now - $( [ -f "$f" ] && date -r "$f" +%s || echo 0 ) ))
  if [ "$age" -ge 30 ]; then
    ( toktrack_prompt_cost > "$f" 2>/dev/null ) &!
  fi
  [ -f "$f" ] && cat "$f"
}
```

(Use `&` instead of `&!` in bash; `&!` is zsh's "disown" form.)

## Make it yours

- Add a label: `printf '🪙 $%.2f' "$cost"`.
- Color it red over a threshold — pairs well with a future `toktrack budget` check.
- Prefer the left prompt? Drop `$(toktrack_prompt_cost)` wherever you like in `PS1` / `PROMPT`.
