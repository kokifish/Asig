#!/usr/bin/env python3
"""Repair Tahoe StatusKit trackedApplications state for a blocked menu-bar app.

This script patches the nested binary plist stored under the `trackedApplications`
key inside ControlCenter's app-group preferences.

Why this exists:
- macOS 26 Tahoe can persist a bad blocked-state for a third-party `NSStatusItem`.
- When that happens, ControlCenter accepts the host, creates a displayable in the
  menu bar, and then immediately moves it to the blocked list and hides it.
- A common corruption mode is:
  1. the target bundle's own tracked entry exists but is not effectively usable, or
  2. a different disallowed tracked entry still contains `menuItemLocations`
     references pointing at the target bundle, and that foreign blocked record wins.

The script repairs both conditions for a chosen bundle id:
- force the target tracked entry's `isAllowed` to `true`
- remove stale `menuItemLocations` references to the target bundle from foreign
  entries whose `isAllowed` is `false`

By default it writes a patched copy. Use `--in-place` only when you are already
running with enough privileges to modify the real ControlCenter plist.

One-shot full repair (patches the system plist in place and restarts cfprefsd and
ControlCenter so the cached bad state is dropped):

    python3 scripts/repair_statuskit_block.py --apply --relaunch-app build/Asig.app

`--apply` is shorthand for `--in-place --restart-controlcenter`; add
`--relaunch-app APP_BUNDLE` to reopen the app afterward. It needs the terminal to
have Full Disk Access (System Settings -> Privacy & Security -> Full Disk Access);
`sudo` does NOT bypass that TCC check, so the script reads/writes the plist
directly as the current user instead of trying sudo.
"""

from __future__ import annotations

import argparse
import plistlib
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

DEFAULT_CONTROL_CENTER_PLIST = (
    Path.home()
    / "Library/Group Containers/group.com.apple.controlcenter"
    / "Library/Preferences/group.com.apple.controlcenter.plist"
)


@dataclass(slots=True)
class RepairStats:
    """Summary of what the repair changed."""

    target_entries: int = 0
    target_entries_forced_allowed: int = 0
    foreign_blocked_entries_touched: int = 0
    foreign_menu_item_refs_removed: int = 0


def parse_args() -> argparse.Namespace:
    """Parse command-line flags."""
    parser = argparse.ArgumentParser(
        description=(
            "Repair Tahoe StatusKit/ControlCenter blocked state for a menu-bar "
            "bundle id."
        )
    )
    parser.add_argument(
        "--bundle-id",
        default="com.kokifish.asig",
        help="Bundle identifier to repair. Default: %(default)s",
    )
    parser.add_argument(
        "--input",
        type=Path,
        default=DEFAULT_CONTROL_CENTER_PLIST,
        help=(
            "Source ControlCenter plist. Default: %(default)s. "
            "Use a copied/exported plist if the system file is protected."
        ),
    )
    parser.add_argument(
        "--output",
        type=Path,
        help=(
            "Patched plist destination. Required unless --in-place is used. "
            "When omitted with --in-place, the source file is overwritten."
        ),
    )
    parser.add_argument(
        "--in-place",
        action="store_true",
        help="Overwrite --input after first creating a backup.",
    )
    parser.add_argument(
        "--backup",
        type=Path,
        help=(
            "Backup path used by --in-place. Default: <input>.bak if omitted. "
            "Ignored unless --in-place is set."
        ),
    )
    parser.add_argument(
        "--restart-controlcenter",
        action="store_true",
        help="After a successful patch, restart cfprefsd and ControlCenter.",
    )
    parser.add_argument(
        "--relaunch-app",
        type=Path,
        help=(
            "Optional app bundle path to reopen after the repair, for example "
            "`build/Asig.app`."
        ),
    )
    parser.add_argument(
        "--apply",
        action="store_true",
        help=(
            "One-shot full repair on the system plist: in-place patch with backup "
            "plus restart of cfprefsd/ControlCenter. Shorthand for "
            "--in-place --restart-controlcenter; add --relaunch-app APP_BUNDLE to "
            "reopen the app afterward. Requires Full Disk Access."
        ),
    )
    parser.add_argument(
        "--print-summary-only",
        action="store_true",
        help="Inspect and print the trackedApplications summary without writing.",
    )
    return parser.parse_args()


def location_bundle_id(node: Any) -> str | None:
    """Extract `bundle._0` if it exists in a trackedApplications node."""
    if not isinstance(node, dict):
        return None
    bundle = node.get("bundle")
    if not isinstance(bundle, dict):
        return None
    raw = bundle.get("_0")
    return raw if isinstance(raw, str) else None


def entry_bundle_id(entry: Any) -> str | None:
    """Return the bundle id of a trackedApplications entry."""
    if not isinstance(entry, dict):
        return None
    return location_bundle_id(entry.get("location"))


def menu_item_locations(entry: dict[str, Any]) -> list[dict[str, Any]]:
    """Return menuItemLocations as a mutable list."""
    raw = entry.get("menuItemLocations")
    if isinstance(raw, list):
        return [item for item in raw if isinstance(item, dict)]
    return []


def summarize_entries(
    entries: list[Any],
    bundle_id: str,
) -> tuple[int, int, int]:
    """Count target entries and stale foreign references for reporting."""
    target_count = 0
    target_allowed = 0
    foreign_ref_count = 0
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        current_bundle_id = entry_bundle_id(entry)
        if current_bundle_id == bundle_id:
            target_count += 1
            if entry.get("isAllowed") is True:
                target_allowed += 1
            continue
        if entry.get("isAllowed") is False:
            refs = menu_item_locations(entry)
            foreign_ref_count += sum(
                1
                for item in refs
                if location_bundle_id(item) == bundle_id
            )
    return target_count, target_allowed, foreign_ref_count


def repair_entries(entries: list[Any], bundle_id: str) -> RepairStats:
    """Repair target allow-state and foreign blocked references in-place."""
    stats = RepairStats()
    for raw_entry in entries:
        if not isinstance(raw_entry, dict):
            continue

        current_bundle_id = entry_bundle_id(raw_entry)
        if current_bundle_id == bundle_id:
            stats.target_entries += 1
            if raw_entry.get("isAllowed") is not True:
                raw_entry["isAllowed"] = True
                stats.target_entries_forced_allowed += 1
            continue

        if raw_entry.get("isAllowed") is not False:
            continue

        original_locations = menu_item_locations(raw_entry)
        if not original_locations:
            continue

        filtered_locations = [
            item
            for item in original_locations
            if location_bundle_id(item) != bundle_id
        ]
        removed_count = len(original_locations) - len(filtered_locations)
        if removed_count == 0:
            continue

        raw_entry["menuItemLocations"] = filtered_locations
        stats.foreign_blocked_entries_touched += 1
        stats.foreign_menu_item_refs_removed += removed_count

    return stats


def load_outer_plist(path: Path) -> dict[str, Any]:
    """Load the outer ControlCenter plist."""
    with path.open("rb") as handle:
        data = plistlib.load(handle)
    if not isinstance(data, dict):
        raise ValueError("outer plist is not a dictionary")
    return data


def load_tracked_entries(outer: dict[str, Any]) -> list[Any]:
    """Decode the nested binary plist stored in `trackedApplications`."""
    tracked = outer.get("trackedApplications")
    if tracked is None:
        raise KeyError("missing `trackedApplications` key")
    if not isinstance(tracked, (bytes, bytearray)):
        raise TypeError(
            "`trackedApplications` is not binary plist data; "
            f"got {type(tracked).__name__}"
        )
    decoded = plistlib.loads(bytes(tracked))
    if not isinstance(decoded, list):
        raise ValueError("decoded `trackedApplications` is not an array")
    return decoded


def store_tracked_entries(outer: dict[str, Any], entries: list[Any]) -> None:
    """Re-encode the nested binary plist back into the outer plist."""
    outer["trackedApplications"] = plistlib.dumps(
        entries,
        fmt=plistlib.FMT_BINARY,
        sort_keys=False,
    )


def write_outer_plist(path: Path, outer: dict[str, Any]) -> None:
    """Write the outer plist using binary format to match the system file."""
    with path.open("wb") as handle:
        plistlib.dump(outer, handle, fmt=plistlib.FMT_BINARY, sort_keys=False)


def restart_controlcenter() -> None:
    """Restart processes that cache the patched ControlCenter state."""
    for command in (["killall", "cfprefsd"], ["killall", "ControlCenter"]):
        subprocess.run(command, check=False)


def relaunch_app(app_bundle: Path) -> None:
    """Reopen the app bundle after the repair."""
    subprocess.run(["open", str(app_bundle)], check=True)


def print_summary(
    bundle_id: str,
    source_path: Path,
    target_count: int,
    target_allowed: int,
    foreign_ref_count: int,
) -> None:
    """Print a compact summary that is easy to reason about in Terminal."""
    print(f"source={source_path}")
    print(f"bundle_id={bundle_id}")
    print(f"target_entries={target_count}")
    print(f"target_entries_already_allowed={target_allowed}")
    print(f"foreign_blocked_refs_to_target={foreign_ref_count}")


def has_selected_mode(args: argparse.Namespace) -> bool:
    """Return True if the caller picked a write/inspect mode."""
    return bool(args.print_summary_only or args.in_place or args.output is not None)


def print_fda_error(path: Path, exc: PermissionError) -> None:
    """Print actionable guidance when the system plist cannot be accessed.

    macOS blocks `~/Library/Group Containers/...` behind a TCC Full Disk Access
    check on the responsible process, so a permission error here means the running
    terminal lacks FDA (not a missing sudo).
    """
    print(f"error: permission denied accessing {path}: {exc}", file=sys.stderr)
    print(
        "       This terminal lacks Full Disk Access. Grant it in\n"
        "         System Settings -> Privacy & Security -> Full Disk Access\n"
        "       to your terminal (Terminal/iTerm/Warp) or to Claude Code, restart\n"
        "       it, and rerun. (sudo does NOT bypass this TCC check.)",
        file=sys.stderr,
    )


def resolve_output_path(args: argparse.Namespace) -> Path:
    """Return the final write target for the patched plist."""
    if args.in_place:
        return args.input
    assert args.output is not None
    return args.output


def maybe_backup_input(args: argparse.Namespace) -> Path | None:
    """Create a backup before in-place modification."""
    if not args.in_place:
        return None
    backup_path = args.backup or args.input.with_suffix(args.input.suffix + ".bak")
    shutil.copy2(args.input, backup_path)
    return backup_path


def main() -> int:
    """CLI entry point."""
    args = parse_args()
    # --apply = --in-place --restart-controlcenter; pair with --relaunch-app to reopen.
    if args.apply:
        args.in_place = True
        args.restart_controlcenter = True

    if not has_selected_mode(args):
        print(__doc__)
        return 0

    try:
        outer = load_outer_plist(args.input)
        entries = load_tracked_entries(outer)
    except PermissionError as exc:  # pragma: no cover - terminal lacks Full Disk Access
        print_fda_error(args.input, exc)
        return 1
    except Exception as exc:  # pragma: no cover - surface exact failure to CLI
        print(f"error: failed to load plist: {exc}", file=sys.stderr)
        return 1

    target_count, target_allowed, foreign_ref_count = summarize_entries(
        entries,
        args.bundle_id,
    )
    print_summary(
        args.bundle_id,
        args.input,
        target_count,
        target_allowed,
        foreign_ref_count,
    )

    if args.print_summary_only:
        return 0

    if target_count == 0 and foreign_ref_count == 0:
        print(
            "error: no target tracked entry and no foreign blocked references "
            "were found for that bundle id",
            file=sys.stderr,
        )
        return 2

    backup_path = None
    output_path = resolve_output_path(args)
    try:
        backup_path = maybe_backup_input(args)
        stats = repair_entries(entries, args.bundle_id)
        store_tracked_entries(outer, entries)
        write_outer_plist(output_path, outer)
    except PermissionError as exc:  # pragma: no cover - terminal lacks Full Disk Access
        print_fda_error(output_path, exc)
        return 1
    except Exception as exc:  # pragma: no cover - surface exact failure to CLI
        print(f"error: failed to write patched plist: {exc}", file=sys.stderr)
        return 1

    print(f"patched={output_path}")
    if backup_path is not None:
        print(f"backup={backup_path}")
    print(f"target_entries_seen={stats.target_entries}")
    print(f"target_entries_forced_allowed={stats.target_entries_forced_allowed}")
    print(f"foreign_blocked_entries_touched={stats.foreign_blocked_entries_touched}")
    print(f"foreign_menu_item_refs_removed={stats.foreign_menu_item_refs_removed}")

    if args.restart_controlcenter:
        restart_controlcenter()
        print("restarted=cfprefsd,ControlCenter")

    if args.relaunch_app is not None:
        relaunch_app(args.relaunch_app)
        print(f"reopened_app={args.relaunch_app}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
