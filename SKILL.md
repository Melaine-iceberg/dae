---
name: file-workspace-design
description: >
  Design and implementation system for a distinctive desktop file manager.
  Use this skill whenever designing, implementing, reviewing, or modifying
  the application's UI, UX, components, layouts, interactions, motion,
  theming, accessibility, or AI-native file management features.
---

# File Workspace Design System

## 1. Product Identity

This application is a desktop file manager designed as a **File Workspace**,
not as a visual clone of Windows Explorer or macOS Finder.

The product should feel:

- Spatial
- Calm
- Expressive
- Tactile
- Focused
- Modern
- Efficient
- Distinctive

Primary design principle:

> Files are content that users work with, not merely rows in a filesystem.

The UI should therefore emphasize:

- context
- spatial relationships
- meaningful grouping
- progressive disclosure
- contextual actions
- visual hierarchy
- fast keyboard interaction

Avoid reproducing the conventional desktop file manager paradigm
unless required for interoperability or user expectations.

---

# 2. Design Philosophy

The design system is inspired by Material 3 Expressive, but MUST NOT
simply reproduce Material 3.

Use Material 3 Expressive as a foundation for:

- design tokens
- dynamic color
- typography hierarchy
- expressive shapes
- state transitions
- motion
- accessibility

Then adapt those principles to a desktop-first file workspace.

The final product should have its own visual identity.

Do not create:

- Windows Explorer with different colors
- macOS Finder with rounded corners
- a web dashboard inside a desktop window
- a generic Material Design application

The product should feel native to its own design language.

---

# 3. Core Information Architecture

The primary navigation model is:

    Overview
    Recent
    Favorites

    Spaces
      Work
      Personal
      Shared
      Archive

    Locations
      Computer
      Cloud
      Network

The concept of a **Space** is central to the product.

A Space is a contextual workspace that may contain:

- local folders
- network locations
- cloud locations
- collections
- saved searches
- favorite files
- saved views

Do not assume that filesystem hierarchy must be the primary
information architecture.

Filesystem hierarchy remains available, but it is not necessarily
the first thing the user sees.

---

# 4. Primary Screens

The application should have the following conceptual screens.

## 4.1 Overview

Overview is the default landing surface.

It should provide:

- recent files
- favorite files
- active Spaces
- useful collections
- quick access to common locations
- lightweight activity information

Do not default directly to a raw filesystem path unless the user
explicitly configures that behavior.

---

## 4.2 Space

A Space represents a contextual working environment.

A Space may contain:

- folders
- files
- collections
- saved searches
- pinned locations

A Space should visually communicate its identity.

---

## 4.3 Folder

Folders use a spatial hierarchy.

Preferred representation:

- expressive folder cards for prominent folders
- compact list/grid for large collections
- optional density modes

Avoid showing every folder as a generic row by default.

---

## 4.4 File View

Files should support:

- list view
- grid view
- compact view
- large preview view

The user should be able to switch density without changing
the underlying information architecture.

---

# 5. Layout Principles

Use a desktop-first layout.

Default structure:

    ┌────────────────────────────────────────────────────┐
    │ Global Header / Search / Commands                  │
    ├──────────────┬─────────────────────────────────────┤
    │              │                                     │
    │ Navigation   │ Main Workspace                      │
    │              │                                     │
    │              │                                     │
    └──────────────┴─────────────────────────────────────┘

The navigation rail/sidebar should be persistent on large screens.

However:

- do not make it excessively wide
- allow collapse
- preserve clear hierarchy
- avoid permanently consuming large screen area

The main content area should always remain the visual focus.

---

# 6. Spatial Surfaces

Use a small number of meaningful surfaces.

Preferred hierarchy:

    App background
        ↓
    Workspace surface
        ↓
    Elevated surface
        ↓
    Floating/contextual surface

Avoid excessive cards.

Not every element should be enclosed in a card.

Cards are for:

- folders
- collections
- important grouped content
- contextual summaries

Do not wrap every row, button, or section in a card.

---

# 7. Design Tokens

All visual values MUST come from semantic tokens.

Do not scatter raw values throughout components.

Use semantic tokens such as:

    color.background
    color.surface
    color.surfaceElevated
    color.surfaceVariant

    color.primary
    color.primaryContainer
    color.onPrimary
    color.onPrimaryContainer

    color.secondary
    color.secondaryContainer

    color.error
    color.warning
    color.success

    color.textPrimary
    color.textSecondary
    color.textTertiary
    color.textDisabled

    border.subtle
    border.strong
    border.focus

    spacing.xs
    spacing.sm
    spacing.md
    spacing.lg
    spacing.xl
    spacing.2xl

    radius.sm
    radius.md
    radius.lg
    radius.xl
    radius.full

    elevation.none
    elevation.low
    elevation.medium
    elevation.high

Do not introduce arbitrary spacing or color values unless the
design system genuinely requires a new semantic token.

---

# 8. Spacing

Use a 4px base spacing system.

Preferred values:

    4
    8
    12
    16
    20
    24
    32
    40
    48
    64

The default content rhythm should primarily use:

    8
    12
    16
    24
    32

Do not use arbitrary values such as 13px, 19px, 27px, etc.
unless required by typography or platform rendering.

---

# 9. Shape Language

The product uses expressive rounded geometry.

Recommended semantic radii:

    sm     8px
    md     12px
    lg     16px
    xl     24px
    full   9999px

Use larger radii for:

- prominent folder cards
- floating panels
- command surfaces
- contextual action bars

Use smaller radii for:

- compact controls
- table rows
- inputs
- menus

Do not round everything.

Shape must communicate hierarchy.

---

# 10. Typography

Typography should be highly legible and information-oriented.

Use a modern system sans-serif unless the platform requires otherwise.

Semantic hierarchy:

    display
    headline
    title
    body
    label
    caption

Primary content should have strong hierarchy.

Important values such as:

- file names
- folder names
- item counts
- dates
- sizes

should be visually differentiated without excessive font-weight changes.

Avoid overly small secondary text.

Do not use typography as decoration.

---

# 11. Color

Use:

**Neutral foundation + expressive accent.**

The background should remain relatively calm.

Accent color should communicate:

- selection
- active navigation
- important actions
- focus
- system state

Avoid rainbow-like file management UI.

File types should not automatically receive aggressive colors.

Do not make every folder or file colorful.

Dynamic color may be supported, but the application must remain
coherent with a neutral theme.

---

# 12. Dark Mode

Dark mode is a first-class theme.

Do not implement dark mode by simply multiplying RGB values.

Dark mode should use semantic surface hierarchy.

Preferred hierarchy:

    background
    ↓
    surface
    ↓
    elevated surface
    ↓
    floating surface

Text contrast must remain accessible.

Avoid pure black backgrounds unless explicitly required.

---

# 13. Navigation

The navigation system uses:

    Overview
    Recent
    Favorites
    Spaces
    Locations

Navigation items should communicate:

- active state
- hover state
- keyboard focus
- disabled state

Active navigation should not rely solely on color.

Use a combination of:

- shape
- surface
- icon
- typography
- subtle accent

Avoid the appearance of a traditional web sidebar.

---

# 14. Toolbar

The toolbar should prioritize:

1. navigation
2. current context
3. search
4. contextual actions

Avoid huge collections of toolbar buttons.

Actions should be:

- discoverable
- grouped
- contextual
- keyboard accessible

Low-frequency actions should move into:

- overflow menu
- command bar
- context menu

---

# 15. Command Bar

The application MUST support a command/search surface.

Default shortcut:

    Cmd/Ctrl + K

The command surface can perform:

- open location
- create folder
- rename
- move
- copy
- copy path
- compress
- delete
- duplicate
- share
- change view
- sort
- filter
- navigate
- execute saved commands

Command search should support fuzzy matching.

Commands should expose keyboard shortcuts where applicable.

---

# 16. Search

Search is a global concept.

The global search should be able to search:

- files
- folders
- Spaces
- locations
- recent items
- metadata

Search should support progressive refinement.

Potential filters:

- type
- date
- size
- location
- tags
- modified time

Search results should preserve context.

Do not make search look like a generic web search engine.

---

# 17. File Cards

Folder/file cards should communicate:

- icon/thumbnail
- name
- type
- metadata
- selection state

Folder cards may display:

- item count
- recent activity
- representative preview

Do not overload cards with metadata.

The primary hierarchy is:

    icon/preview
    ↓
    name
    ↓
    useful metadata

---

# 18. File List

The list view should prioritize scanning speed.

Default hierarchy:

    Name
    Modified
    Type
    Size

Additional metadata should be configurable.

Rows should have:

- hover state
- selected state
- focus state
- disabled state
- drag target state

Do not use excessive borders.

Use spacing and subtle separators instead.

---

# 19. Selection

Selection is a core interaction.

Support:

- single selection
- multi-selection
- range selection
- keyboard selection
- drag selection

Selection must be visually obvious.

Do not rely only on a background color.

Preferred signals:

- accent container
- selection indicator
- icon state
- contextual toolbar

When one or more files are selected, contextual actions may appear.

Example:

    3 selected
    Open
    Move
    Copy
    Share
    Delete
    More

The contextual action surface should appear only when useful.

---

# 20. Contextual Action Bar

Contextual actions should follow the selection.

Do not permanently occupy toolbar space with actions that are
only relevant after selection.

Contextual actions may include:

- open
- preview
- move
- copy
- rename
- share
- compress
- delete

Destructive actions must be visually differentiated.

---

# 21. Preview

Preview should be contextual rather than permanently consuming
a large portion of the workspace.

Preferred behavior:

- inline preview for lightweight content
- floating preview for focused content
- dedicated preview surface for complex documents

Preview should show:

- content
- filename
- type
- size
- modified time
- location
- available actions

Avoid permanently displaying a Finder-like inspector panel.

---

# 22. Drag and Drop

Drag and drop should feel physical.

Visual states should communicate:

    draggable
    dragging
    valid target
    invalid target
    drop active

Use motion to establish spatial continuity.

When possible, preserve the user's mental model:

    item
      ↓
    destination

Do not abruptly teleport content between locations without feedback.

---

# 23. Motion

Motion is functional, not decorative.

Use motion to communicate:

- hierarchy
- causality
- continuity
- state
- spatial relationship

Preferred motion characteristics:

- responsive
- subtle
- spring-like
- short
- interruptible

Avoid:

- long animations
- gratuitous bouncing
- cinematic transitions
- motion that blocks productivity

Typical interactions:

Folder open:
expand → transition → content appears

Selection:
surface responds → contextual actions appear

Drag:
item lifts → destination reacts → item settles

Delete:
item moves toward removal state → disappears

Navigation:
current context transitions into new context

Respect reduced-motion accessibility preferences.

---

# 24. Elevation

Elevation should indicate hierarchy, not decoration.

Use:

- low elevation for floating controls
- medium elevation for command surfaces
- higher elevation for modal or critical overlays

Avoid heavy shadows.

Use surface contrast before shadows.

---

# 25. Icons

Icons should be:

- consistent
- simple
- recognizable
- visually balanced

Do not use icons merely for decoration.

Icons should communicate actions or object identity.

File-type icons should be visually consistent.

Avoid mixing unrelated icon libraries.

---

# 26. Empty States

Empty states should explain:

1. what is empty
2. why it is empty
3. what the user can do next

Example:

    No recent files

    Files you open or edit will appear here.

    [ Browse files ]

Do not leave large blank areas without explanation.

---

# 27. Loading States

Prefer:

- skeletons
- progressive content
- lightweight placeholders

Avoid full-screen spinners unless the entire application is blocked.

Do not block unrelated parts of the UI while loading one location.

---

# 28. Error States

Errors should be:

- understandable
- actionable
- contextual

Example:

    Couldn't access this location

    The network connection may have been interrupted.

    [ Retry ]    [ Open another location ]

Do not expose raw filesystem exceptions as the primary user message.

Technical error details may be available through an advanced disclosure.

---

# 29. Accessibility

Accessibility is mandatory.

Support:

- keyboard navigation
- visible focus
- screen readers
- sufficient contrast
- reduced motion
- scalable text
- semantic labels
- non-color-only state communication

All interactive elements must have:

- accessible name
- focus behavior
- keyboard behavior
- disabled behavior where applicable

Selection, errors, warnings, and status must not rely exclusively
on color.

---

# 30. Keyboard First

The application is a desktop productivity tool.

Keyboard interaction is a first-class feature.

Support common patterns:

    Cmd/Ctrl + K      Command bar
    Cmd/Ctrl + F      Search
    Enter             Open
    Space             Preview/select where appropriate
    F2                Rename
    Cmd/Ctrl + C      Copy
    Cmd/Ctrl + X      Cut
    Cmd/Ctrl + V      Paste
    Delete            Delete
    Escape            Cancel/close
    Cmd/Ctrl + A      Select all

Respect platform conventions where necessary.

Do not force web-style keyboard interactions onto desktop users.

---

# 31. Context Menus

Context menus should be concise.

Prioritize:

    Open
    Preview
    Rename
    Copy
    Move
    Share
    Compress
    Delete

Secondary actions may be placed under:

    More

Do not create enormous context menus containing every possible
filesystem operation.

---

# 32. AI-Native Design

The application should be designed so AI capabilities can be
introduced without breaking the existing information architecture.

AI is an assistant layer, not a replacement for the filesystem.

Potential AI operations:

- natural-language file search
- semantic search
- summarize documents
- organize files
- identify duplicates
- suggest collections
- rename groups of files
- explain storage usage
- find related files
- perform multi-step file operations

AI operations must clearly distinguish:

    user intent
    AI interpretation
    proposed action
    executed action

---

# 33. AI State Model

AI interactions may have states such as:

    idle
    understanding
    searching
    planning
    waiting_for_confirmation
    executing
    completed
    partially_completed
    failed

These states must be visible but not distracting.

Do not use a generic "AI is thinking..." animation everywhere.

Use meaningful status.

Example:

    Searching 1,248 files…

    Found 36 likely duplicates.

    [ Review ]    [ Organize ]

---

# 34. AI Confirmation

Destructive or large-scale AI operations require explicit confirmation.

Examples:

    Delete 247 files
    Move 1.4 GB
    Rename 83 files

The UI should communicate:

- scope
- affected items
- destination
- irreversible consequences

Never hide destructive AI actions behind a single ambiguous button.

---

# 35. AI Results

AI-generated results should remain editable.

Examples:

    Suggested folder:
    "2026 Product Launch"

    34 files

    [ Review ] [ Rename ] [ Apply ]

The user should be able to inspect and modify the proposal before
execution.

AI should propose whenever confidence or consequences warrant it.

---

# 36. Progressive Disclosure

Do not show everything at once.

Primary actions should be immediately visible.

Secondary actions should appear through:

- context menus
- overflow menus
- command bar
- contextual toolbars
- preview surfaces

The UI should remain calm even when the underlying filesystem
is complex.

---

# 37. Density Modes

Support at least:

    Comfortable
    Compact

Optional:

    Spacious

Density changes spacing and row height, not information architecture.

Power users should be able to increase information density.

---

# 38. View Modes

Support:

    List
    Grid
    Compact Grid

Potential future:

    Columns
    Timeline
    Gallery
    Saved View

Do not assume one representation is universally optimal.

---

# 39. Responsive Desktop Behavior

The application should adapt to:

- small desktop windows
- large monitors
- ultrawide monitors
- split-screen layouts

When space decreases:

1. reduce secondary information
2. collapse navigation
3. simplify toolbar
4. preserve primary content

Do not simply scale everything down.

---

# 40. Platform Independence

The design language should remain visually independent from:

- Windows Explorer
- macOS Finder
- GNOME Files
- KDE Dolphin

Platform conventions may still be respected for:

- keyboard shortcuts
- window behavior
- file dialogs
- accessibility
- system menus

Platform compatibility must not force platform visual imitation.

---

# 41. Avoid These Patterns

Never default to:

- Windows Explorer clone
- Finder clone
- generic SaaS dashboard
- excessive card layouts
- giant rounded rectangles everywhere
- excessive gradients
- excessive blur
- excessive glass effects
- excessive shadows
- rainbow file icons
- toolbar button overload
- permanent inspector panels
- unnecessary animations
- tiny secondary text
- color-only states
- arbitrary spacing
- arbitrary component variants

---

# 42. Material 3 Expressive Adaptation

Use M3 Expressive concepts selectively.

Use:

- expressive shapes
- dynamic color
- semantic tokens
- motion
- strong hierarchy
- responsive state transitions

Do NOT directly copy:

- Android navigation patterns
- Android-specific component dimensions
- mobile bottom navigation
- mobile-first spacing
- Android-specific dialogs
- Material icons if they conflict with product identity

The final system must feel desktop-native but not platform-native.

---

# 43. Liquid Glass Inspiration

Liquid Glass may be used as inspiration for:

- translucent overlays
- depth
- floating surfaces
- visual separation
- spatial hierarchy

Do not turn the entire UI into glass.

Glass should be reserved for:

- command surfaces
- temporary overlays
- contextual panels
- floating controls

The main workspace should remain calm and readable.

---

# 44. Component Architecture

Components should be organized by semantic role.

Suggested hierarchy:

    primitives/
        Button
        IconButton
        Text
        Surface
        Divider
        Input

    navigation/
        Sidebar
        NavigationItem
        Breadcrumb
        SpaceSwitcher

    files/
        FileItem
        FolderCard
        FileGrid
        FileList
        FileThumbnail
        SelectionIndicator

    workspace/
        WorkspaceHeader
        Toolbar
        ContextualActionBar
        Preview
        EmptyState

    search/
        SearchField
        SearchResults
        SearchFilters
        CommandBar

    overlays/
        ContextMenu
        Dialog
        Popover
        Tooltip
        Toast

    ai/
        AICommand
        AIStatus
        AIProposal
        AIConfirmation
        AIResult

Prefer composition over large monolithic components.

---

# 45. Component States

Every interactive component should explicitly consider:

    default
    hover
    pressed
    focused
    selected
    disabled
    loading
    error

Components that represent asynchronous operations should also
consider:

    pending
    success
    partial
    failed

Do not implement only the happy path.

---

# 46. State Management and Visual State

Visual states should derive from application state.

Do not create fake visual states disconnected from the actual
filesystem or operation state.

Examples:

Selection UI must derive from selection state.

Upload/progress UI must derive from actual operation progress.

AI execution state must derive from the actual task state.

---

# 47. Performance

The design system must not create unnecessary rendering cost.

File managers may display thousands or millions of filesystem items.

Prefer:

- virtualization
- lazy thumbnails
- incremental rendering
- memoized item rendering
- progressive previews
- asynchronous metadata loading

Visual richness must not compromise filesystem performance.

---

# 48. Large Collections

For large directories:

- default to compact density
- virtualize lists/grids
- defer expensive previews
- avoid rendering hidden items
- avoid excessive DOM/component depth

Do not render thousands of complex cards simultaneously.

---

# 49. Thumbnails

Thumbnails should be:

- cached
- lazy-loaded
- asynchronously generated
- replaceable with placeholders

Use consistent thumbnail geometry.

Avoid layout shifts while thumbnails load.

---

# 50. Design Review Checklist

When implementing or reviewing a UI, ask:

### Identity

- Does this look like our product?
- Could this be mistaken for Windows Explorer?
- Could this be mistaken for Finder?
- Could this be mistaken for a generic web dashboard?

### Hierarchy

- Is the primary content obvious?
- Are secondary actions hidden appropriately?
- Is information density appropriate?

### Interaction

- Does selection feel physical?
- Are contextual actions available?
- Is keyboard interaction supported?
- Does motion explain state changes?

### Accessibility

- Is focus visible?
- Are states understandable without color?
- Does reduced motion work?
- Are interactive elements keyboard accessible?

### Performance

- Does this scale to large directories?
- Are previews lazy?
- Is rendering virtualized where necessary?

### AI

- Is AI state visible?
- Are proposed operations distinguishable from executed operations?
- Are destructive actions confirmed?
- Can users review and edit AI proposals?

---

# 51. Implementation Rules for Coding Agents

When modifying UI code:

1. Inspect existing design tokens before introducing new values.
2. Reuse existing components before creating new components.
3. Prefer semantic tokens over raw CSS values.
4. Preserve keyboard accessibility.
5. Preserve reduced-motion behavior.
6. Preserve existing selection semantics.
7. Do not introduce platform-specific visual patterns without a reason.
8. Do not add decorative UI that does not improve hierarchy or interaction.
9. Keep file operations visually connected to their real state.
10. Consider large directory performance before adding visual effects.
11. Keep AI actions explicit and reversible where possible.
12. Do not introduce a new component variant when composition is sufficient.

---

# 52. When Adding a New Component

Before creating a new component:

1. Search for an existing component with similar semantics.
2. Check whether an existing primitive can be composed.
3. Determine whether the new component is reusable.
4. Define its semantic states.
5. Define keyboard behavior.
6. Define accessibility behavior.
7. Define light/dark behavior.
8. Define reduced-motion behavior.
9. Define loading/error behavior if asynchronous.
10. Add tests for important interaction states.

---

# 53. When Reviewing UI Code

Reject implementations that:

- hardcode arbitrary colors
- hardcode arbitrary spacing
- ignore dark mode
- ignore keyboard navigation
- remove focus indicators
- use color as the only state signal
- add excessive animations
- create unnecessary modal dialogs
- duplicate existing components
- introduce inconsistent corner radii
- render huge file collections without virtualization
- hide destructive operations behind ambiguous controls

---

# 54. Visual Quality Bar

The UI should feel:

    Calm rather than busy.
    Expressive rather than decorative.
    Spatial rather than flat.
    Tactile rather than mechanical.
    Fast rather than animated.
    Focused rather than feature-heavy.
    Distinctive rather than platform-cloned.

The ultimate goal is:

> A desktop file workspace that feels familiar enough to be immediately
> usable, but distinctive enough that users cannot mistake it for
> Windows Explorer or macOS Finder.

---

# 55. Priority Order

When design decisions conflict, prioritize in this order:

    1. Usability
    2. Accessibility
    3. Performance
    4. Information hierarchy
    5. Interaction clarity
    6. Product identity
    7. Visual expressiveness
    8. Decorative detail

Never sacrifice usability or performance merely to achieve a visual effect.

---

# 56. Final Design Principle

The product should not ask:

> "How can we make a better file explorer?"

It should ask:

> "How can we make working with files feel like working in a
> modern spatial workspace?"

Every new feature should reinforce that idea.
