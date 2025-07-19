- [Download](#download)
- [Concepts](#concepts)
  - [Map](#map)
  - [Movement](#movement)
  - [Action](#action)
      - [Normal and priority](#normal-and-priority)
      - [Configuration](#configuration)
  - [Linked Key & Linked Action](#linked-key-%26-linked-action)
  - [Rotation Modes](#rotation-modes)
    - [Auto-mobbing](#auto-mobbing)
    - [Ping Pong](#ping-pong)
  - [Platforms Pathing](#platforms-pathing)
  - [Navigation](#navigation)
  - [Run/stop Cycle](#run%2Fstop-cycle)
  - [Capture Modes](#capture-modes)
  - [Familiars Swapping](#familiars-swapping)
  - [Panic Mode](#panic-mode)
  - [Elite Boss Spawns Behavior](#elite-boss-spawns-behavior)
- [Video guides](#video-guides)
- [Showcase](#showcase)
  - [Rotation](#rotation)
  - [Auto Mobbing & Platforms Pathing](#auto-mobbing-%26-platforms-pathing)
  - [Rune Solving](#rune-solving)

## Download

- Head to the [Github Release](https://github.com/sasanquaa/komari/releases)
- Download `app-release-gpu.zip` or `app-release-cpu.zip` and extract it
- Run the exe file

## Concepts

#### Map

- Map is automatically detected but must be created manually by providing a name
- The created map is saved and can be selected again later
- Any actions preset created in the detected map is saved to that map only

The arcs are only for visual and do not represent the actual moving path. However, it does represent
the order of one action to another as indicated by the number.

TODO: Update image

#### Movement
Bot default movement without platforms pathing is really simple:
1. Moves horizontally first to match `x` of a destination
2. Then performs fall/up jump/grapple to match `y` of a destination

If `x` is close enough to a destination (`close enough` currently means distance than `25`, subject
to change), the bot will walk instead of double jump.

#### Characters
- `Characters` tab is used to change key bindings, set up buffs,...
- Can be created separately for each character
- Character is saved globally and not affected by the detected map
- There are four sections:
  - `Key bindings`: For general in-game key bindings
  - `Buffs`: For automatic buffs configuration
  - `Fixed actions`: Actions that are shared across all maps, useful for buffs or one-time skills
  - `Others`: Other game-related configurations for the character

For supported buffs in the configuration, the bot relies on detecting buffs on the top-right corner.
From v0.12, `Rope lift` skill can now be disabled. If not provided, the bot will just try to up jump.

![Buffs](https://github.com/sasanquaa/komari/blob/master/.github/images/buffs.png?raw=true)

#### Action
There are two types of action:
- Move - Moves to a location on the map
- Key - Uses a key with or without location

##### Normal And Priority

An action is categorized into normal and priority actions. A priority action can override a normal
action and force the player to perform the former. The normal action is not completely overriden
and is only delayed until the priority action is complete.

Currently, priority actions consist of `Erda Shower off cooldown` and `Every milliseconds` actions.

For `Erda Shower off cooldown` action to work, the skill Erda Shower must be assigned to
the quick slots, with Action Customization toggled on and **visible** on screen. The skill
should also be casted when using this type of action or the actions will be re-run.

![Erda Shower](https://github.com/sasanquaa/komari/blob/master/.github/images/erda.png?raw=true)

##### Configuration

Move action configurations:
- `Adjust`: 
  - Whether the actual position should be as close as possible to the specified position 
  - Enabling this option will cause `Disable walking` in the `Characters` tab to be ignored and make the character walks
- `X`: The horizontal x position to move to
- `X random range`: Random x between `[x - range, x + range]` to move to
- `Y`: The vertical y position to move to
- `Wait after move`: The milliseconds to wait after moving (e.g. for looting)
- `Linked action`:
  - See [linked action](#linked-key-%26-linked-action)
  - Can only be enabled if it is not the first action or the action list is non-empty

Key action configurations:
- `Positioned`: Whether this key action has a position
- `X`, `X random range`, `Y`, `Adjust`, `Linked action`: Same as move action
- `Key`: The key to use
- `Use count`: Number of times to use the key
- `Has link key`: Optionally enable link key (useful for [combo classes](#linked-key-%26-linked-action))
- `Queue to front`:
  - Applicable only to priority actions
  - When set, this action can override other non-`Queue to front` priority action
  - The overriden priority action is not lost but delayed like normal action
  - Useful for action such as `press attack after x milliseconds even while moving`
  - Cannot override linked action
- `Use direction`: The direction to use the key
- `Use with`:
  - `Any` - Performs as the bot sees appropriate
  - `Stationary` - Performs an action only when standing on ground (for buffs)
  - `DoubleJump` - Performs an action with double jump
- `Wait before`/`Wait after`:
  - Wait for the specified amount of millseconds after/before using the key
  - Waiting is applied on each repeat of `Use count`
- `Wait random range`: Applies randomization to the delay in the range `delay - range` to `delay + range`

Actions added in the list below can be reordered by clicking the up/down icons.

![Actions](https://github.com/sasanquaa/komari/blob/master/.github/images/actions.png?raw=true)

#### Linked Key & Linked Action
Linked key and linked action are useful for combo-oriented class such as Blaster, Cadena, Ark, Mercedes,...
Animation cancel timing is specific to each class. As such, the timing is approximated and provided in the configuration, so make sure you select the appropriate one.

For linked key, there are four link types:
- `Before` - Uses the link key before the actual key (e.g. for Cadena, Chain Arts: Thrash is the link key)
- `AtTheSame` - Uses the link key at the same time as the actual key (probably only Blaster skating needs this)
- `After` - Uses the link key after the actual key (e.g. for Blaster, Weaving/Bobbing is the link key)
- `Along` - Uses the link key along with the actual key while the link key is being held down (e.g. for in-game Combo key)

Note that even though `AtTheSame` would send two keys simultaneously, *the link key will be send first*. When the configured
class is set to Blaster, the performing action has `After` link type and the link key is not `Jump Key`, an extra `Jump Key` will be sent for cancelling Bobbing/Weaving. The same effect can also be achieved through linked action.

As for `Along` link type, the timing is fixed and does not affected by class type.

Linked action is for linking action(s) into a chain. Linked action can be created by ticking `Linked action` checkbox action below any other action. The first action is the start of the actions chain:

TODO: Image

Linked action cannot be overriden by any other type of actions once it has started executing regardless of whether the action is a normal or priority action.

#### Rotation Modes
Rotation mode specifies how to run the actions and affects **only** normal actions. It can be changed in the `Rotation` section in the `Actions` tab. There are three modes:
- `StartToEnd` - Runs actions from start to end in the order added and repeats
- `StartToEndThenReverse` - Runs actions from start to end in the order added and reverses (end to start)
- `AutoMobbing` - All added normal actions are ignored and, instead, detects a random mob within bounds to hit
- `PingPong` - All added normal actions are ignored and, instead, double jumps and uses key until hitting the bound edges

For other priority actions:
- `Every milliseconds` actions run out of order
- `Erda Shower off cooldown` actions run in the order added same as `StartToEnd`

##### Auto-mobbing
Auto-mobbing is feature to hit random mobs detected on screen. It can be enabled by changing rotation mode to `AutoMobbing`. When `AutoMobbing` is used:
- Setting the bounds to inside the minimap is required so that the bot will not wrongly detect out of bounds mobs
- The bounds should be the rectangle where you can move around (two edges of the map)
- While this mode ignores all normal actions, it is still possible to use other priority actions
- For platforms pathing, see [Platforms Pathing](#platforms-pathing)
- If platforms are provided:
  - Use platforms as pathing points for more accurate mob positions and better movement:
    - If not provided, the bot will try to figure out over time what `y` the player can stand on, which can be slow
    - Providing platforms help figuring out these `y` immediately
  - Try to detect "gaps" between platforms to ignore invalid mob positions
  - Note that it is only encouraged and not neccessary to add platforms when using auto-mobbing but then the bot will rely solely on randomness

(From v0.18.0)
Auto-mobbing now follows a fixed clockwise order based on the provided bound like the below image. This has improved
auto-mobbing mob count even in larger maps. Previously, auto-mobbing suffers from too much randomness and unable to move
around much in larger maps.

How it works:
1. The bound is divided into four quads
2. The player will move to each of this quad in a clockwise order
3. The player will only detect and hit mobs inside each of this quad (outside of this quad is ignored)
4. If there is no more mob, move to the next quad

Red arrow's tail indicates current quad and its head indicates the next quad to move to.

![Auto-mobbing](https://github.com/sasanquaa/komari/blob/master/.github/images/automobbing.png?raw=true)

##### Ping Pong
Added in v0.12:
- All added normal actions are ignored but still possible to use other priority actions similar to `AutoMobbing`
- Player double jumps and uses key until hitting the bound edges, then reverses in the other direction
- Forces the player to always try and stay inside the bound
- If already inside bound:
  - Has a chance to grapple/up jump if below bound mid `y`
  - Has a chance to fall down if above bound mid `y`
  - Within the distance of `9` from the bound mid `y`, randomization (grapple/up jump/fall down) will not happen
- Simpler than `AutoMobbing`, can achieve higher mob count and useful for class that mostly just double jumps and spams attack (e.g. Night Walker)

#### Platforms Pathing
Platforms pathing is currently only supported for auto-obbing and rune solving. This feature exists to help
pathing around platforms with or without `Rope Lift` skill. To use this feature, add all the map's platforms starting
from the ground level.

When adding platforms, hot keys can be used to add platforms more quickly. And it is encouraged to add platforms when
used for auto-mobbing as it can help auto-mobbing as documented in [Auto-mobbing](#auto-mobbing).

#### Navigation
(Added in v0.19)

Navigation is a feature to help the player moves to a designated map automatically. The current system only supports
navigating through map with a visible minimap. There are two main concepts:
- Path:
  - Represents a snapshot of a minimap that contains minimap and its name images
  - Contains point(s) (e.g. portal coordinates) that will transition to another path upon moving
- Point:
  - Represents a transition to a different path

Navigation paths can be created following this procedure:
1. Open `Navigation` tab
2. Note down a list of maps you want to navigate (e.g. Esfera Base Camp -> Esfera Mirror-touched Sea 3)
3. Go to each map and ensure the minimap is currently detectable
4. Click `Add path`:
    - Automatically capture the current minimap and its name images
    - Used for matching against other paths to know the player current location
5. Go to a portal inside the map and click `Add point`:
    - Automatically record the portal coordinate to be used for navigating
    - Can optionally select the next path this point will transition to
6. Repeat from 3. until all paths are added
7. Select a created map and attach a path to it through `Attached path`

After following the above procedure, when clicking `Start`, the bot will try to navigate to the attached path first before 
rotating the actual actions. Useful for:
- Bot [run/stop Cycle](#run%2Fstop-cycle) that will stop, go town for a specified duration and start again
- Navigate back to the original map if accidental map changing occurs

This system is currently experimental and subject to changes. It has many limitations in that it cannot do interaction-based
navigation (e.g. Maple Guide, teleport, ...) or minimap is not visible. But can serve as a foundation for implementing other 
features that may require navigation.

![Navigation](https://github.com/sasanquaa/komari/blob/master/.github/images/navigation.png?raw=true)

#### Run/stop Cycle
(Added in v0.19)

Added in the `Settings` tab under `Run/stop cycle` section. When `Enabled`, the bot will run for the specified `Run duration` and then stop, go to town, AFK for the specified `Stop duration` and back to running. Using this feature requires the key binding 
for `To town` is set and navigation paths for the selected map are available.

#### Capture Modes
`Capture` section in the `Settings` tab can be used to change how the bot captures game images. There are three capture modes, the first two are similar to what you see in OBS:
- `BitBlt` - The default capture mode that works for GMS
  - GMS Stargazer update made this mode stopped working on some machines and should be changed to the below method
- `Windows 10 (1903 and up)` - The alternative capture mode for Windows 10 that works for TMS/MSEA
- `BitBltArea` - Captures a fixed area on the screen
  - This capture mode is useful if you are running the game inside something else or want to use fixed capture area (e.g. a VM, capture card (?) or Sunshine/Moonlight)
  - The capture area can stay behind the game but it cannot be minimized
  - **When the game resizes (e.g. going to cash shop), the capture area must still contain the game**
  - **When using this capture mode, key inputs will also be affected:**
    - **Make sure the window on top of the capture area is focused by clicking it for key inputs to work**
    - For example, if you have Notepad on top of the game and focused, it will send input to the Notepad instead of the game

You can also directly select which window to capture via `Handle`.

#### Familiars Swapping
(From v0.13)
`Familiars` section in the `Settings` tab is a feature to help periodically checking currently equipped familiar levels and swapping them out with new familiars if the any of the equipped ones level is maxed:
- `Swap check milliseconds`: Check for swapping every `X` milliseconds
- `Swappable slots`:
  - `All`: All slots can be swapped
  - `Last`: Only last slot can be swapped
  - `SecondAndLast`: Only second and last slots can be swapped
- `Can swap rare familiars`: Familiar with rare rarity will be included when swapping
- `Can swap epic familiars`: Familiar with epic rarity will be included when swapping

Familiars swapping supports scrolling the familiar cards list to find more selectable cards. But for best experience, the cards list should contain selectable cards immediately without scrolling.

After swapping, it will save the setup and cause the familiar buff to turn off. Therefore, the familiar buff in the `Buffs` tab should also be turned on.

**This feature currently assumes all familiar slots have already been expanded. Also make sure your key binding to open the familiar menu is set when using this feature.**

#### Panic Mode
(From v0.14)
Panic mode can be enabled in the `Settings` tab. Once enabled, if another player (friend, guild, stranger) appears in the same map for 15 seconds, it will enter `Panicking` state and cycle through channels until a channel without any other player is found.

(From v0.18)
If `Stop actions on fail or map changed` is enabled, the bot will stop upon failure and go to town. 

Requires setting up key bindings for `To town` and `Change channel` first.

#### Elite Boss Spawns Behavior
(From v0.17, previously known as `Change Channel On Elite Boss`)

Added `Elite boss spawns behavior` in `Characters` tab with two behavior types:
- `CycleChannel`: Queues a channel change (`Panicking` state) action when an elite boss appears
- `UseKey`: Uses a key when elite boss appears (e.g. useful for origin skill)

## Video guides
From v0.16, due to UI change, the first two videos are now outdated but the general concepts still apply.

1. [Basic operations](https://youtu.be/8X2CKS7bnHY?si=3yPmVPaMsFEyDD8c)
2. [Auto-mobbing and platforms pathing](https://youtu.be/8r2duEz6278?si=HTHb8WXh6L7ulCoE)
3. Rotation modes, linked key and linked actions - TODO
    - [Clockwise rotation example](https://youtu.be/-glx3b0jGEY?si=nuEDmIQTuiz3LtIq) 

## Showcase (These showcases are from v0.1)
#### Rotation
https://github.com/user-attachments/assets/3c66dcb9-7196-4245-a7ea-4253f214bba6

(This Blaster rotation was before Link Key & Link Action were added)

https://github.com/user-attachments/assets/463b9844-0950-4371-9644-14fad5e1fab9

#### Auto Mobbing & Platforms Pathing
https://github.com/user-attachments/assets/3f087f83-f956-4ee1-84b0-1a31286413ef

#### Rune Solving
https://github.com/user-attachments/assets/e9ebfc60-42bc-49ef-a367-3c20a1cd00e0
