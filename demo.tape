Output assets/demo.gif
Set Width 1200
Set Height 600
Set FontSize 14
Set Theme "Homebrew"

# Launch
Type "toktrack"
Enter
Sleep 1500ms

# Overview: move through sources with j
Type "j"
Sleep 300ms
Type "j"
Sleep 300ms
Type "j"
Sleep 1000ms

# Return to the first source and open SourceDetail
Type "k"
Sleep 200ms
Type "k"
Sleep 200ms
Type "k"
Sleep 200ms
Enter
Sleep 1500ms

# SourceDetail: scroll Daily
Type "k"
Sleep 200ms
Type "k"
Sleep 200ms
Type "k"
Sleep 200ms
Type "k"
Sleep 200ms
Type "k"
Sleep 1000ms

# Model breakdown popup
Enter
Sleep 2000ms
Escape
Sleep 500ms

# Weekly view
Type "w"
Sleep 1500ms

# Monthly view
Type "m"
Sleep 1500ms

# Return to Overview
Escape
Sleep 1000ms

# Open the second source (Codex)
Type "j"
Sleep 300ms
Enter
Sleep 500ms

# Switch from Monthly to Daily
Type "d"
Sleep 1500ms

# Scroll Codex Daily
Type "k"
Sleep 200ms
Type "k"
Sleep 200ms
Type "k"
Sleep 1000ms

# Codex Model breakdown popup
Enter
Sleep 2000ms
Escape
Sleep 500ms

# Return to Overview
Escape
Sleep 1000ms

# Tab 2: Stats
Type "2"
Sleep 1500ms

# Tab 3: Models
Type "3"
Sleep 1500ms

# Quit (Ctrl+C, then y)
Ctrl+C
Sleep 500ms
Type "y"
