#!/usr/bin/env python3
"""Validate actual edit destinations for Claude and Codex hook payloads."""
import json
from pathlib import Path
import subprocess
import sys


def edit_paths(payload):
    tool_input = payload.get('tool_input')
    if not isinstance(tool_input, dict):
        raise ValueError('missing edit input')
    file_path = tool_input.get('file_path')
    if isinstance(file_path, str) and file_path.strip():
        return [file_path]
    patch = tool_input.get('command') or tool_input.get('patch') or tool_input.get('input')
    if not isinstance(patch, str):
        raise ValueError('missing file_path or patch')
    lines = patch.strip().splitlines()
    if not lines or lines[0] != '*** Begin Patch' or lines[-1] != '*** End Patch':
        raise ValueError('unrecognized patch envelope')
    paths = []
    for line in lines[1:-1]:
        for prefix in ('*** Add File: ', '*** Update File: ', '*** Delete File: ', '*** Move to: '):
            if line.startswith(prefix):
                path = line[len(prefix):]
                if not path.strip():
                    raise ValueError('empty patch destination')
                paths.append(path)
                break
    if not paths:
        raise ValueError('no edit destinations')
    return paths


def check(payload):
    cwd = payload.get('cwd') or str(Path.cwd())
    if not isinstance(cwd, str) or not Path(cwd).is_absolute() or not Path(cwd).is_dir():
        raise ValueError('invalid working directory')
    for raw in edit_paths(payload):
        path = Path(raw)
        # Resolve symlinks before finding the nearest existing parent of a new file.
        target = (path if path.is_absolute() else Path(cwd) / path).resolve()
        directory = target.parent
        while not directory.exists():
            directory = directory.parent
        result = subprocess.run(
            ['git', '-C', str(directory), 'rev-parse', '--is-inside-work-tree'],
            text=True, capture_output=True, check=False,
        )
        if result.returncode:
            # Non-repository local files are outside this repository branch guard.
            if 'not a git repository' in result.stderr:
                continue
            raise ValueError('cannot inspect destination repository')
        branch = subprocess.run(
            ['git', '-C', str(directory), 'symbolic-ref', '--quiet', '--short', 'HEAD'],
            text=True, capture_output=True, check=False,
        )
        if branch.returncode == 1:
            raise ValueError('edit destination is on detached HEAD')
        if branch.returncode:
            raise ValueError('cannot inspect destination branch')
        if branch.stdout.strip() in ('main', 'master'):
            raise ValueError('edit destination is on a protected branch; use a feature worktree')


def main():
    try:
        payload = json.load(sys.stdin)
        if not isinstance(payload, dict):
            raise ValueError('invalid hook payload')
        check(payload)
    except (ValueError, OSError, RuntimeError) as error:
        print(f'BLOCK: {error}', file=sys.stderr)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main())
