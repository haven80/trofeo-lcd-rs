# Linux Install with Optional Autostart and Upgrade/Rollback Script

[← Back to the README](../README.md) · [All the docs](./README.md)

This was tested on Ubuntu 24.04, using udev rules to grant user access to the specific Trofeo Screen USB device vendor/product ID. Should work in Debian distros like Ubuntu 26.04, Mint, etc.  
This has not yet been tested on other linux distros based on Arch or Fedora, some prerequisites, dependencies and packages might differ.

It is possible for the purpose of running machines with dual boot, to use the same "trofeo.conf" and "trofeo.conf.bak" files both in Windows and Linux, making it a seamless transitions between OSes where the Trofeo screen configuration and layout stays the same if you wish so.
Of course this also works for machines running Linux exclusively. 

For the purpose of this guide the example is shown as installing the trofeo-lcd-rs.3.0.0, change the file name to the version you are installing.

> [!NOTE]
> * For simple install with no intention to upgrade: Follow step 1-3 and then jump to step 7.
> * For simple install+autostart with intention upgrade: Follow step 1-3 and then jump to step 7-8.
> * For full install+autostart+upgrade/rollback script: Follow step 1-10 only extracting the source code on step 3.

> [!TIP]
> For the upgrade/rollback script, mentioned in step 4, consider this following folders as separate roles for the script to work as intended:

	$HOME/Downloads/trofeo-lcd-rs-<version>/             # extracted source
	$HOME/Applications/trofeo/versions/<version>/        # installed, built version
	$HOME/Applications/trofeo/current -> versions/...    # active version
	$HOME/Applications/trofeo/config-backup/             # saved config copies

> [!WARNING]
> Sometimes the terminal output can be redirected, showing nothing. If `printf 'OUTPUT TEST\n' shows nothing, use the command bellow to fix it.

	exec 1>/dev/tty

You can confirm it with:

	printf 'OUTPUT TEST\n'

If it prints "OUTPUT TEST" then you know it's fixed and you can go back to the commands that were not outputting messages.



**Step 1. Install the prerequisites**

	sudo apt update

	sudo apt install libudev-dev pkg-config pkgconf build-essential playerctl pipewire-pulse

On Fedora based distros:

	sudo dnf install systemd-devel pkgconf-pkg-config

On Arch based distros:

	sudo pacman -S systemd pkgconf 

> [!WARNING]
> (Fedora and Arch commands yet untested, please adapt and search for similar commands if you're running a distro with those bases)

build-essential provides the C compiler and linker tools that Rust may need, including cc.

On Debian-based systems, libudev-dev is the package commonly needed for Rust builds that use hidapi. 
If you already installed the package, check whether pkg-config can see it:

	pkg-config --modversion libudev

If that prints a version you can now proceed and step 3 or 5 will run ok.



**Step 2. Install Rust and enable PipeWire support**

	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
	source "$HOME/.cargo/env"

If you see no default linker 'cc' was found in your PATH, install the compiler and linker tools(this shouldn't happen if you already installed it in step 1):

	sudo apt install build-essential

Then reopen the terminal or run source "$HOME/.cargo/env" again and retry.
If your system uses PipeWire, enable its PulseAudio-compatible socket:

	systemctl --user enable --now pipewire-pulse.socket



**Step 3. Extract and build the source**

Extract the release source(Downloaded "trofeo-lcd-rs-<version>.tar.gz" file).

> [!NOTE]
> **Ignore the rest of this step 3 if you wish to configure the folders and script for update and/or rollback later on this tutorial, proceed to step 4.  
> Proceed with this step 3 if you wish to do a standalone install with no intentions to update trofeo-lcd-rs in the future.**

Change into its directory. In this example version 3.0.0 is used, change it to the version you downloaded:

	cd $HOME/Downloads/trofeo-lcd-rs-3.0.0

Or navigate to the downloaded folder and right click and select "Open in Terminal" and run:

	cargo build --release

> [!NOTE]
> **From here jump to Step 7 to make the screen work, and to Step 8 if you wish to configure autostart, changing the mentioned folders on step 8 to the folder where you extracted the Source code (tar.gz) file to).**


**Step 4. Create the application directories**

If you didn't create then before step one, create them now:

	mkdir -p $HOME/Applications/trofeo/current
	mkdir -p $HOME/Applications/trofeo/versions
	mkdir -p $HOME/Applications/trofeo/config-backup



**Step 5. Install the clean-install version under "versions" folder**

Copy the extracted source into its own installed version folder. For a clean 3.0.0 install:

	cp -a $HOME/Downloads/trofeo-lcd-rs-3.0.0 $HOME/Applications/trofeo/versions/3.0.0

Build the installed copy:

	cd $HOME/Applications/trofeo/versions/3.0.0
	cargo build --release

Cargo may display warnings about unused variables or code. Those warnings don’t necessarily mean the build failed; check whether it says `Finished` and returns to the prompt.
cargo fix is optional, not required to build or run the app. If you want to let Cargo apply suggested changes, run it from the source directory. A release archive may not include Git metadata, so use --allow-no-vcs:

From inside the folder in terminal run:

	cargo fix --bin trofeo_lcd -p trofeo_lcd --allow-no-vcs

The project may have more than one executable, for example the screen, and the config file. If you specifically want to check suggestions for the configuration program, the command you test is:

	cargo fix --bin trofeo_config -p trofeo_lcd --allow-no-vcs

A successful cargo fix may make no changes; it is not a prerequisite for using the program. The release build creates target/release/trofeo_lcd and target/release/trofeo_config.

If you want to run it, both trofeo_lcd and/or trofeo_config GUI, you can run from 'trofeo-lcd-rs-<version>' root folder in terminal:

	./target/release/trofeo_lcd
	./target/release/trofeo_config

> [!NOTE]
> If it prints the the following error:

	./target/release/trofeo_lcd Error: USB error: Access denied (insufficient permissions) Caused by: Access denied (insufficient permissions) 
 
The program can see the USB device, but your user account doesn’t have permission to open it.  
It will run using sudo command only for testing but we don't want that, we want the program to run with user based permissions, and not administrator based permissions.

The lasting fix is a udev rule granting your user access to that specific device you will find in Step 7.


**Step 6. Create the active-version link**

For the clean-install example, point current to the installed copy:

	ln -sfn $HOME/Applications/trofeo/versions/3.0.0 $HOME/Applications/trofeo/current

Verify it:

	readlink -f $HOME/Applications/trofeo/current

For this example, it should print a path ending in:

	Applications/trofeo/versions/3.0.0

Which means that the syslink is pointing to the version in the folder 3.0.0

> [!WARNING]
> Don't skip the active-version link or the upgrade/rollback script won't work.

**Step 7. Grant USB access to the display**

Like mentioned in Step 5.
Find the device in the output, which will look like ID 1234:abcd or ID 1234:1234. Then check whether the app works when run as root:

	sudo ./target/release/trofeo_lcd

If that works, the lasting fix is a udev rule granting your user access to that specific device. Avoid making all USB devices accessible to everyone. A rule can match the device’s vendor/product IDs and assign it to a group; the exact rule depends on the IDs shown by `lsusb`.

	lsusb

For example in the Trofeo 11.3 used for this tutorial, it showed as:

	Bus 003 Device 003: ID 0416:5408 Winbond Electronics Corp. USBDISPLAY

So if you run:

	ls -l /dev/bus/usb/003/003

It outputs something like: `crw-rw---- 1 root:root 189, 265 nov 21 17:26 /dev/bus/usb/003/003`

Where `root:root` is intended to be `user:root` and user, is your user username in the system. 

The device appears as Winbond Electronics Corp. USBDISPLAY, ID 0416:5408 in the Trofeo 11.3, but yours might show something different so adapt the following command to the name and ID of your Trofeo Screen. The rule that worked for this guide assigned the USB device to this machine specific user.  
Create it with your own user username replacing "USER" by the login name on the computer in `OWNER="**INSERT YOUR USERNAME HERE**"`, and you might have to change `ATTR{idVendor}=="0416"` and `ATTR{idProduct}=="5408"` if your screen shows something different than  Winbond Electronics Corp. USBDISPLAY, ID 0416:5408:  
Adapt the following command with your own user and/or your trofeo idVendor and idProduct:

	echo 'SUBSYSTEM=="usb", ATTR{idVendor}=="0416", ATTR{idProduct}=="5408", MODE="0660", TAG+="uaccess"' | sudo tee /etc/udev/rules.d/99-trofeo-lcd.rules <<'EOF'SUBSYSTEM=="usb", ENV{DEVTYPE}=="usb_device", ATTR{idVendor}=="0416", ATTR{idProduct}=="5408", OWNER="INSERT YOUR USERNAME HERE", MODE="0600"EOF

Reload udev rules:

	sudo udevadm control --reload-rules
	sudo udevadm trigger

Unplug and reconnect the panel, then check the current device path using:

	lsusb

and

	ls -l /dev/bus/usb/003/NNN

(replace NNN with its current device number, as it changes the device ID after disconnecting and reconnecting).
The owner should now be your user.

It should output something like: `crw-rw---- 1 user:root 189, 265 nov 21 17:26 /dev/bus/usb/003/006` where user is your user name.

Then try:

./target/release/trofeo_lcd

If it runs without sudo, means that your user has access.
This limits access to your user and that specific USB device. Udev rules can set a device node’s owner and permissions.
Do not run the app with `sudo` as your user has now permissions to run it and have access to the USB device.

**Step 8. Test the app and set up desktop autostart**

Test the app and configuration editor as your regular user:

	$HOME/Applications/trofeo/current/target/release/trofeo_lcd

	$HOME/Applications/trofeo/current/target/release/trofeo_config

To start the app automatically at desktop login, create:

	mkdir -p $HOME/.config/autostart

	nano $HOME/.config/autostart/trofeo-lcd.desktop

Add:

	ini
	
	[Desktop Entry]Type=ApplicationName=Trofeo LCDExec=$HOME/Applications/trofeo/current/target/release/trofeo_lcdTerminal=falseX-GNOME-Autostart-enabled=true

And save it.

Because the entry uses current, it follows that symlink when you switch versions in the future.



**Step 9. Create the updater**

The updater below accepts the path to an extracted source folder. It copies that folder into versions, builds it, backs up both config files from the active version, copies them into the new version, and switches current only after a successful build.
Create or replace the updater by running this whole block in the terminal:

	#!/usr/bin/env bash
	set -Eeuo pipefail

	BASE="${TROFEO_BASE:-"$HOME/Applications/trofeo"}"
	VERSIONS="$BASE/versions"
	CURRENT="$BASE/current"
	BACKUPS="$BASE/config-backup"

	if [[ $# -ne 1 ]]; then
	    echo "Usage: $0 /path/to/extracted/trofeo-lcd-rs-version-folder" >&2
	    exit 2
	fi

	SOURCE="$(realpath "$1")"
	if [[ ! -d "$SOURCE" || ! -f "$SOURCE/Cargo.toml" ]]; then
	    echo "Error: source folder must exist and contain Cargo.toml: $SOURCE" >&2
	    exit 1
	fi

	if [[ ! -L "$CURRENT" || ! -d "$CURRENT" ]]; then
	    echo "Error: current must be a working symlink to the active version." >&2
	    exit 1
	fi

	ACTIVE="$(realpath "$CURRENT")"
	CONFIG="$ACTIVE/target/release/trofeo.conf"
	CONFIG_BAK="$ACTIVE/target/release/trofeo.conf.bak"

	if [[ ! -f "$CONFIG" || ! -f "$CONFIG_BAK" ]]; then
	    echo "Error: active version is missing trofeo.conf or trofeo.conf.bak." >&2
	    echo "No update was made." >&2
	    exit 1
	fi

	VERSION="$(basename "$SOURCE")"
	DEST="$VERSIONS/$VERSION"

	mkdir -p "$VERSIONS" "$BACKUPS"

	if [[ -e "$DEST" ]]; then
	    echo "Error: destination already exists: $DEST" >&2
	    echo "No update was made. Check its contents before removing or renaming it." >&2
	    exit 1
	fi

	echo "Copying $SOURCE to $DEST"
	cp -a "$SOURCE" "$DEST"

	echo "Building $VERSION..."
	if ! (cd "$DEST" && cargo build --release); then
	    echo "Build failed. The active version was not changed." >&2
	    echo "The copied folder remains at: $DEST" >&2
	    exit 1
	fi

	NEW_RELEASE="$DEST/target/release"
	if [[ ! -x "$NEW_RELEASE/trofeo_lcd" ]]; then
	    echo "Error: build did not produce target/release/trofeo_lcd." >&2
	    echo "The active version was not changed." >&2
	    exit 1
	fi

	STAMP="$(date +%Y%m%d-%H%M%S)"
	ACTIVE_NAME="$(basename "$ACTIVE")"

	echo "Backing up and copying configuration..."
	cp -p "$CONFIG" "$BACKUPS/${ACTIVE_NAME}-${STAMP}-trofeo.conf"
	cp -p "$CONFIG_BAK" "$BACKUPS/${ACTIVE_NAME}-${STAMP}-trofeo.conf.bak"
	cp -p "$CONFIG" "$NEW_RELEASE/trofeo.conf"
	cp -p "$CONFIG_BAK" "$NEW_RELEASE/trofeo.conf.bak"

	TEMP_LINK="$BASE/.current.new.$$"
	trap 'rm -f "$TEMP_LINK"' EXIT
	ln -s "$DEST" "$TEMP_LINK"
	mv -Tf "$TEMP_LINK" "$CURRENT"
	trap - EXIT

	echo "Update successful. current now points to: $DEST"


	chmod +x $HOME/Applications/trofeo/update-trofeo.sh

Then you can run:

	bash -n $HOME/Applications/trofeo/update-trofeo.sh

If "bash -n" succeeds, it prints nothing and returns to the prompt.



**Step 10. Updade and/or rollback**

To update, extract the new release into a new Downloads folder; then run the updater with the extracted folder. For example:

	$HOME/Applications/trofeo/update-trofeo.sh $HOME/Downloads/trofeo-lcd-rs-3.0.1

Use that example only, and change it to whatever version you are updating to, in this example is the 3.0.1 release. After a successful update, check:

	readlink -f $HOME/Applications/trofeo/current

It should point to the new folder under Applications/trofeo/versions/. If the build fails, the updater leaves current pointing at the previous version. To roll back, point current to the earlier installed version, for example:

	ln -sfn $HOME/Applications/trofeo/versions/3.0.0 $HOME/Applications/trofeo/current

This will point it back to the version 3.0.0 that exists inside the "versions" folder, so you will effectively roll back from 3.0.1 to 3.0.0 in this specific example.

Optional:

If you get warnings like this:

	warning: `trofeo_lcd` (bin "trofeo_config") generated 6 warnings (run `cargo fix --bin "trofeo_config" -p trofeo_lcd` to apply 6 suggestions)
	warning: `trofeo_lcd` (bin "trofeo_lcd") generated 13 warnings (run `cargo fix --bin "trofeo_lcd" -p trofeo_lcd` to apply 2 suggestions)

Navigate to folder current:

	cd $HOME/Applications/trofeo/current

So you can run the fix from a folder which has the newer running version and the ‘Cargo.toml’ file in it.

Then run:

	cargo fix --bin trofeo_lcd -p trofeo_lcd --allow-no-vcs

And:

	cargo fix --bin trofeo_config -p trofeo_lcd --allow-no-vcs

A successful cargo fix may make no changes; it is not a prerequisite for using the program. The release build creates target/release/trofeo_lcd and target/release/trofeo_config.

This is the same as mentioned in step 5.

To check if everything is pointing to the new updated version run the following command:

	readlink -f $HOME/Applications/trofeo/current

In this example updating from 3.0.0 to 3.0.1 it showed the following:

	readlink -f $HOME/Applications/trofeo/current
`$HOME/Applications/trofeo/versions/trofeo-lcd-rs-3.0.1`

This confirms the version running is pointed to the version inside the folder “versions” 3.0.1 now.

With this script it will preserve both the original downloaded version in the ‘Download’ folder, you can keep it or delete it, and it will also preserve the file ‘trofeo.conf’ and ‘trofeo.conf.bak’ inside the folder ‘$HOME/Applications/trofeo/current/target/release’, so you have your settings and screens just like you had from the previous version.  
