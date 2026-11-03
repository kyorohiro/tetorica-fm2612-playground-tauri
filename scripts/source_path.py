"""Locate parent-checkout sources while retaining historical snapshot keys."""
from pathlib import Path


MOVED_ROOTS = {'web': 'runtime', 'node': 'node', 'cli': 'cli',
               'native': 'native', 'wasm': 'wasm'}


def source_path(root, name):
    root, name = Path(root), Path(name)
    # Snapshot directories and older checkouts keep their original layout.
    # Select one layout for the whole checkout; a missing moved file must fail
    # rather than silently using an obsolete copy from the old location.
    if (root / 'tetorica_src').is_dir() and not name.is_absolute() and name.parts:
        moved = MOVED_ROOTS.get(name.parts[0])
        if moved is not None:
            return root / 'tetorica_src' / moved / Path(*name.parts[1:])
    return root / name
