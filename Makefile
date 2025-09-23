TARGET = alacritty

ASSETS_DIR = extra
RELEASE_DIR = target/release
MANPAGE = $(ASSETS_DIR)/man/alacritty.1.scd
MANPAGE-MSG = $(ASSETS_DIR)/man/alacritty-msg.1.scd
MANPAGE-CONFIG = $(ASSETS_DIR)/man/alacritty.5.scd
MANPAGE-CONFIG-BINDINGS = $(ASSETS_DIR)/man/alacritty-bindings.5.scd
TERMINFO = $(ASSETS_DIR)/alacritty.info
COMPLETIONS_DIR = $(ASSETS_DIR)/completions
COMPLETIONS = $(COMPLETIONS_DIR)/_alacritty \
	$(COMPLETIONS_DIR)/alacritty.bash \
	$(COMPLETIONS_DIR)/alacritty.fish

APP_NAME = Alacritty.app
APP_TEMPLATE = $(ASSETS_DIR)/osx/$(APP_NAME)
APP_DIR = $(RELEASE_DIR)/osx
APP_BINARY = $(RELEASE_DIR)/$(TARGET)
APP_BINARY_DIR = $(APP_DIR)/$(APP_NAME)/Contents/MacOS
APP_EXTRAS_DIR = $(APP_DIR)/$(APP_NAME)/Contents/Resources
APP_COMPLETIONS_DIR = $(APP_EXTRAS_DIR)/completions
APP_ICON_ASSET = $(ASSETS_DIR)/logo/alacritty.icon
APP_ICON_NAME = alacritty
APP_ASSETCATALOG_DIR = $(APP_DIR)/assetcatalog
APP_NOTARIZE_ARCHIVE = $(APP_DIR)/$(APP_NAME).zip
BUILD_CONFIG = .build-config

-include $(BUILD_CONFIG)

APP_CODESIGN_IDENTITY ?= $(ALACRITTY_CODESIGN_IDENTITY)
APP_CODESIGN_IDENTITY ?= -
APP_CODESIGN_FLAGS ?= $(ALACRITTY_CODESIGN_FLAGS)
APP_CODESIGN_FLAGS ?=
APP_NOTARIZE_PROFILE ?= $(ALACRITTY_NOTARIZE_PROFILE)
APP_NOTARIZE_PROFILE ?=

DMG_NAME = Alacritty.dmg
DMG_DIR = $(RELEASE_DIR)/osx

vpath $(TARGET) $(RELEASE_DIR)
vpath $(APP_NAME) $(APP_DIR)
vpath $(DMG_NAME) $(APP_DIR)

all: help

help: ## Print this help message
	@grep -E '^[a-zA-Z._-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

binary: $(TARGET)-native ## Build a release binary
binary-universal: $(TARGET)-universal ## Build a universal release binary
$(TARGET)-native:
	MACOSX_DEPLOYMENT_TARGET="10.11" cargo build --release
$(TARGET)-universal:
	MACOSX_DEPLOYMENT_TARGET="10.11" cargo build --release --target=x86_64-apple-darwin
	MACOSX_DEPLOYMENT_TARGET="10.11" cargo build --release --target=aarch64-apple-darwin
	@lipo target/{x86_64,aarch64}-apple-darwin/release/$(TARGET) -create -output $(APP_BINARY)

app: $(APP_NAME)-native ## Create an Alacritty.app
app-universal: $(APP_NAME)-universal ## Create a universal Alacritty.app
$(APP_NAME)-%: $(TARGET)-%
	@mkdir -p $(APP_BINARY_DIR)
	@mkdir -p $(APP_EXTRAS_DIR)
	@mkdir -p $(APP_COMPLETIONS_DIR)
	@scdoc < $(MANPAGE) | gzip -c > $(APP_EXTRAS_DIR)/alacritty.1.gz
	@scdoc < $(MANPAGE-MSG) | gzip -c > $(APP_EXTRAS_DIR)/alacritty-msg.1.gz
	@scdoc < $(MANPAGE-CONFIG) | gzip -c > $(APP_EXTRAS_DIR)/alacritty.5.gz
	@scdoc < $(MANPAGE-CONFIG-BINDINGS) | gzip -c > $(APP_EXTRAS_DIR)/alacritty-bindings.5.gz
	@tic -xe alacritty,alacritty-direct -o $(APP_EXTRAS_DIR) $(TERMINFO)
	@cp -fRp $(APP_TEMPLATE) $(APP_DIR)
	@cp -fp $(APP_BINARY) $(APP_BINARY_DIR)
	@cp -fp $(COMPLETIONS) $(APP_COMPLETIONS_DIR)
	@rm -rf $(APP_ASSETCATALOG_DIR)
	@mkdir -p $(APP_ASSETCATALOG_DIR)
	@xcrun actool "$(APP_ICON_ASSET)" \
		--app-icon "$(APP_ICON_NAME)" \
		--compile "$(APP_ASSETCATALOG_DIR)" \
		--output-partial-info-plist "$(APP_ASSETCATALOG_DIR)/asset-info.plist" \
		--platform macosx --target-device mac \
		--minimum-deployment-target 10.11
	@cp -fp "$(APP_ASSETCATALOG_DIR)/Assets.car" "$(APP_EXTRAS_DIR)/Assets.car"
	@cp -fp "$(APP_ASSETCATALOG_DIR)/alacritty.icns" "$(APP_EXTRAS_DIR)/alacritty.icns"
	@/usr/libexec/PlistBuddy -c "Merge $(APP_ASSETCATALOG_DIR)/asset-info.plist" "$(APP_DIR)/$(APP_NAME)/Contents/Info.plist" >/dev/null 2>&1 || true
	@rm -rf $(APP_ASSETCATALOG_DIR)
	@touch -r "$(APP_BINARY)" "$(APP_DIR)/$(APP_NAME)"
	@codesign --remove-signature "$(APP_DIR)/$(APP_NAME)"
	@codesign --force --deep $(APP_CODESIGN_FLAGS) --sign "$(APP_CODESIGN_IDENTITY)" "$(APP_DIR)/$(APP_NAME)"
	@set -e; if [ -n "$(APP_NOTARIZE_PROFILE)" ]; then \
		ditto -c -k --sequesterRsrc --keepParent "$(APP_DIR)/$(APP_NAME)" "$(APP_NOTARIZE_ARCHIVE)"; \
		xcrun notarytool submit "$(APP_NOTARIZE_ARCHIVE)" --keychain-profile "$(APP_NOTARIZE_PROFILE)" --wait; \
		xcrun stapler staple "$(APP_DIR)/$(APP_NAME)"; \
		xcrun stapler validate "$(APP_DIR)/$(APP_NAME)"; \
		rm -f "$(APP_NOTARIZE_ARCHIVE)"; \
	fi
	@echo "Created '$(APP_NAME)' in '$(APP_DIR)'"

dmg: $(DMG_NAME)-native ## Create an Alacritty.dmg
dmg-universal: $(DMG_NAME)-universal ## Create a universal Alacritty.dmg
$(DMG_NAME)-%: $(APP_NAME)-%
	@echo "Packing disk image..."
	@ln -sf /Applications $(DMG_DIR)/Applications
	@hdiutil create $(DMG_DIR)/$(DMG_NAME) \
		-volname "Alacritty" \
		-fs HFS+ \
		-srcfolder $(APP_DIR) \
		-ov -format UDZO
	@echo "Packed '$(APP_NAME)' in '$(APP_DIR)'"

install: $(INSTALL)-native ## Mount disk image
install-universal: $(INSTALL)-native ## Mount universal disk image
$(INSTALL)-%: $(DMG_NAME)-%
	@open $(DMG_DIR)/$(DMG_NAME)

.PHONY: app binary clean dmg install $(TARGET) $(TARGET)-universal

clean: ## Remove all build artifacts
	@cargo clean
