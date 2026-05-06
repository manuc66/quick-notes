#!/bin/bash

# Quick Notes global shortcut wrapper
# Run this to focus the existing window

# Launch the app directly - single-instance will handle focusing
/home/manu/sources/quick-notes/target/debug/quick-notes "$@"

# If you want system-wide Ctrl+Alt+N, set up in your desktop environment:
# - KDE: System Settings > Shortcuts > Custom Shortcuts
# - GNOME: Settings > Keyboard Shortcuts
# - Add custom shortcut: Ctrl+Alt+N -> /home/manu/sources/quick-notes/focus-quick-notes.sh