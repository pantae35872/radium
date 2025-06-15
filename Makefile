ifeq (test,$(firstword $(MAKECMDGOALS)))
  RUN_ARGS := $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS))
  $(eval $(RUN_ARGS):;@:)
endif
ifeq (test-run,$(firstword $(MAKECMDGOALS)))
	STILL_TESTING := 1
else 
	STILL_TESTING := 0
endif
ifeq (release,$(firstword $(MAKECMDGOALS))) 
	RELEASE := 1
  BUILD_MODE := release
else ifeq (tftp-release,$(firstword $(MAKECMDGOALS))) 
	RELEASE := 1
  BUILD_MODE := release
else 
  BUILD_MODE := debug
endif

CRATES := $(patsubst %/,%,$(wildcard src/*/))

.PHONY: debug release clean run test-run test dbg-run force_rebuild dbg-run-no-dbg check tftp-debug tftp-release $(CRATES)
.DEFAULT_GOAL := debug

NAME := radium
BUILD_DIR := build
ISO_DIR := $(BUILD_DIR)/iso
ISO_FILE := $(BUILD_DIR)/os.iso
FAT_IMG := $(BUILD_DIR)/fat.img
DISK_FILE := disk.img
DWARF_FILE := $(abspath $(BUILD_DIR)/dwarf.baker)
TFTP_DIR := /srv/tftp

# Dependency files
KERNEL_DEPS := $(wildcard $(BUILD_DIR)/x86_64/$(BUILD_MODE)/*.d)
BAKER_DEPS := $(wildcard $(BUILD_DIR)/release/baker.d)
BOOTLOADER_DEPS := $(wildcard $(BUILD_DIR)/x86_64-unknown-uefi/$(BUILD_MODE)/*.d)

KERNEL_OPTS_DEPS := src/kernel/src/boot/boot.asm src/kernel/src/boot/trampoline.asm

# Binaries
KERNEL_BIN := $(abspath $(BUILD_DIR)/x86_64/$(BUILD_MODE)/$(NAME))
BOOTLOADER_BIN := $(abspath $(BUILD_DIR)/x86_64-unknown-uefi/$(BUILD_MODE)/$(NAME)-bootloader.efi)
BAKER_BIN := $(abspath $(BUILD_DIR)/release/baker)
KERNEL_BUILD_BIN := $(abspath $(BUILD_DIR)/kernel.bin)
BUILD_MODE_FILE := $(BUILD_DIR)/.build_mode
BOOT_INFO := bootinfo.toml
TEST_BOOT_INFO := test_bootinfo.toml
KERNEL_FONT := kernel-font.ttf

OVMF := OVMF.fd

ifeq ($(BUILD_MODE), $(shell cat $(BUILD_MODE_FILE) 2>/dev/null))
    BUILD_MODE_CHANGED := 0
else
    BUILD_MODE_CHANGED := 1
endif

-include $(KERNEL_DEPS)
-include $(BOOTLOADER_DEPS)
-include $(BAKER_DEPS)

QEMU_FLAGS := -m 1G -bios OVMF.fd \
	-drive id=disk,file=$(DISK_FILE),if=none,format=qcow2 -device ahci,id=ahci \
	-device ide-hd,drive=disk,bus=ahci.0 -boot d -machine kernel_irqchip=split \
	-smp cores=4 -usb -device usb-ehci,id=ehci -device usb-tablet,bus=usb-bus.0 \
	-no-reboot -serial stdio \

KVM_FLAGS := -enable-kvm -cpu host,+rdrand,+sse,+mmx

$(BUILD_DIR):
	mkdir $(BUILD_DIR)

$(ISO_DIR):
	mkdir $(ISO_DIR)


ifneq ($(STILL_TESTING),1)
ifeq ($(BUILD_MODE_CHANGED),1)
force_rebuild:

$(KERNEL_BIN): force_rebuild
$(KERNEL_BUILD_BIN): force_rebuild
$(DWARF_FILE): force_rebuild
$(BOOTLOADER_BIN): force_rebuild
$(FAT_IMG): force_rebuild
$(ISO_FILE): force_rebuild
endif
endif

$(DISK_FILE):
	qemu-img create -f qcow2 $(DISK_FILE) 1G

$(OVMF):
	bash -c 'cd vendor/edk2 && source edksetup.sh && make -C BaseTools && build -a X64 -t GCC5 -p OvmfPkg/OvmfPkgX64.dsc -b RELEASE'
	cp vendor/edk2/Build/OvmfX64/RELEASE_GCC5/FV/OVMF.fd $(OVMF)

run: $(DISK_FILE) $(OVMF)
	qemu-system-x86_64 $(QEMU_FLAGS) $(KVM_FLAGS) -display sdl -cdrom $(BUILD_DIR)/os.iso

dbg-run: $(DISK_FILE) $(OVMF)
	@echo $$$$ > /tmp/dbg_make_pid.txt; \
	qemu-system-x86_64 $(QEMU_FLAGS) -cpu Skylake-Client -display sdl -cdrom $(BUILD_DIR)/os.iso -S -s

dbg-run-no-dbg: $(DISK_FILE) $(OVMF)
	qemu-system-x86_64 $(QEMU_FLAGS) -cpu Skylake-Client -display sdl -cdrom $(BUILD_DIR)/os.iso -device isa-debug-exit,iobase=0xf4,iosize=0x04

$(BUILD_MODE_FILE): $(BUILD_DIR) force_rebuild
	@echo $(BUILD_MODE) > $(BUILD_MODE_FILE)

$(KERNEL_FONT):
	wget https://www.1001fonts.com/download/font/open-sans.regular.ttf
	mv open-sans.regular.ttf kernel-font.ttf

$(DWARF_FILE): $(KERNEL_BUILD_BIN) $(BAKER_BIN)
	$(BAKER_BIN) $(KERNEL_BUILD_BIN) $(DWARF_FILE)

$(KERNEL_BIN): $(KERNEL_OPTS_DEPS)
ifneq ($(STILL_TESTING),1)
	cd src/kernel && RUST_BACKTRACE=1 cargo build $(if $(RELEASE),--release,) --features panic_exit
endif

$(KERNEL_BUILD_BIN): $(KERNEL_BIN)
ifneq ($(STILL_TESTING),1)
	cp $(KERNEL_BIN) $(KERNEL_BUILD_BIN)
endif

$(BOOTLOADER_BIN):
	cd src/bootloader && cargo build $(if $(RELEASE),--release,) 
	cp $(BOOTLOADER_BIN) $(BUILD_DIR)/BOOTX64.EFI

$(BAKER_BIN):
	cd src/baker && cargo build --release

$(FAT_IMG): $(BOOT_INFO) $(BUILD_DIR) $(KERNEL_FONT) $(DWARF_FILE) $(BOOTLOADER_BIN)
	dd if=/dev/zero of=$(FAT_IMG) bs=1M count=64 status=none
	mkfs.vfat -F32 $(FAT_IMG)
	mmd -i $(FAT_IMG) ::/EFI ::/EFI/BOOT ::/boot
ifneq ($(STILL_TESTING),1)
	mcopy -D o -i $(FAT_IMG) $(BOOT_INFO) ::/boot
else
	mcopy -D o -i $(FAT_IMG) $(TEST_BOOT_INFO) ::/boot
	mmove -D o -i $(FAT_IMG) boot/$(TEST_BOOT_INFO) boot/$(BOOT_INFO) 
endif
	mcopy -D o -i $(FAT_IMG) $(KERNEL_FONT) $(DWARF_FILE) ::/boot
	mcopy -D o -i $(FAT_IMG) $(KERNEL_BUILD_BIN) ::/boot 
	mcopy -D o -i $(FAT_IMG) $(BUILD_DIR)/BOOTX64.EFI ::/EFI/BOOT

$(ISO_FILE): $(FAT_IMG) $(ISO_DIR)
	cp $(FAT_IMG) $(ISO_DIR)
	xorriso -as mkisofs -R -f -e fat.img -no-emul-boot -o $(BUILD_DIR)/os.iso $(ISO_DIR)

release: $(BUILD_MODE_FILE) $(OVMF) $(ISO_FILE)
debug: $(BUILD_MODE_FILE) $(OVMF) $(ISO_FILE) 
tftp-debug: $(BUILD_MODE_FILE) $(FAT_IMG)
	sudo rm -rf $(TFTP_DIR)/*
	sudo mcopy -o -i $(FAT_IMG) -s ::* $(TFTP_DIR)
tftp-release: $(BUILD_MODE_FILE) $(FAT_IMG)
	sudo rm -rf $(TFTP_DIR)/*
	sudo mcopy -o -i $(FAT_IMG) -s ::* $(TFTP_DIR)

# Get called by test_run.sh
test-run: $(DISK_FILE) $(BUILD_MODE_FILE) $(OVMF) $(ISO_FILE) 
	@echo test > $(BUILD_MODE_FILE)
	qemu-system-x86_64 $(QEMU_FLAGS) $(KVM_FLAGS) -cdrom $(BUILD_DIR)/os.iso -device isa-debug-exit,iobase=0xf4,iosize=0x04 -display none ; \
		status=$$?; \
		if [ $$status -ne 33 ]; then exit $$status; else exit 0; fi

check: $(CRATES)

$(CRATES):
	@cd $@ && \
		if [ "$@" == "src/bakery" ]; then \
			cargo check --message-format=json --no-default-features --features alloc ; \
		else \
			cargo check --message-format=json --all-targets ;\
			exit 0; \
		fi

test: 
	cd src/kernel && cargo test --features testing $(RUN_ARGS)

clean:
	rm -rf $(BUILD_DIR)
	rm -rf $(DISK_FILE)
	rm -rf $(KERNEL_FONT)
