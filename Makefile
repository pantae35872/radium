ifeq (test,$(firstword $(MAKECMDGOALS)))
  RUN_ARGS := $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS))
  $(eval $(RUN_ARGS):;@:)
endif
ifeq (release,$(firstword $(MAKECMDGOALS))) 
	RELEASE := 1
  BUILD_MODE := release
else 
  BUILD_MODE := debug
endif

.PHONY: debug release clean run make-test-kernel test-run test disk update font ovmf dbg-run
.DEFAULT_GOAL := debug

NAME := radium
BUILD_DIR := build
ISO_DIR := $(BUILD_DIR)/iso
FAT_IMG := $(BUILD_DIR)/fat.img
KERNEL_BIN := $(BUILD_DIR)/kernel.bin
OSRUNNER_SOURCES := $(shell find src/os-runner/src -name '*.rs')
KERNEL_SOURCES := $(shell find src/kernel/src -name '*.rs') $(shell find src/common/src -name '*.rs') src/kernel/build.rs src/kernel/linker.ld
BOOTLOADER_SOURCES := $(shell find src/bootloader/src -name '*.rs') $(shell find src/common/src -name '*.rs')
OSRUNNER_BIN := $(BUILD_DIR)/os-runner
BOOTLOADER_BIN := $(BUILD_DIR)/bootx64.efi
BUILD_MODE_FILE := $(BUILD_DIR)/.build_mode
BOOT_INFO := bootinfo.toml
KERNEL_FONT := kernel-font.ttf

ifeq ($(BUILD_MODE), $(shell cat $(BUILD_MODE_FILE) 2>/dev/null))
    BUILD_MODE_CHANGED := 0
else
    BUILD_MODE_CHANGED := 1
endif

ifeq ($(BUILD_MODE_CHANGED), 1)
    force_rebuild:
			@echo "Build mode have change rebuilding the entire os"
			rm -rf $(BUILD_DIR) 
else
    force_rebuild:
			@echo "Build mode unchanged"
endif

$(BUILD_DIR):
	mkdir $(BUILD_DIR)
	@echo $(BUILD_MODE) > $(BUILD_MODE_FILE)

$(ISO_DIR):
	mkdir $(ISO_DIR)

disk:
	qemu-img create -f qcow2 disk.img 1G

font:
	wget https://www.1001fonts.com/download/font/open-sans.regular.ttf
	mv open-sans.regular.ttf kernel-font.ttf

ovmf:
	wget https://github.com/clearlinux/common/raw/master/OVMF.fd

run: 
	qemu-system-x86_64 -cdrom $(BUILD_DIR)/os.iso -m 1G -bios OVMF.fd \
	-drive id=disk,file=disk.img,if=none,format=qcow2 -device ahci,id=ahci \
	-device ide-hd,drive=disk,bus=ahci.0 -boot d -machine kernel_irqchip=split \
	-no-reboot -enable-kvm -cpu host,+rdrand -serial stdio -display sdl 

dbg-run:
	qemu-system-x86_64 -cdrom $(BUILD_DIR)/os.iso -m 1G -bios OVMF.fd \
	-drive id=disk,file=disk.img,if=none,format=qcow2 -device ahci,id=ahci \
	-device ide-hd,drive=disk,bus=ahci.0 -boot d -machine kernel_irqchip=split \
	-no-reboot -serial stdio -display gtk -S -s

test-run:
	qemu-system-x86_64 -cdrom $(BUILD_DIR)/os.iso -m 1G -bios OVMF.fd -serial stdio \
	-drive id=disk,file=disk.img,if=none,format=qcow2 -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
	-device ahci,id=ahci -device ide-hd,drive=disk,bus=ahci.0 -boot d -machine kernel_irqchip=split \
	-no-reboot -enable-kvm -cpu host,+rdrand -display none 

$(OSRUNNER_BIN): $(OSRUNNER_SOURCES) $(BUILD_DIR) 
	cd src/os-runner && cargo build --release --quiet
	cp src/os-runner/target/release/os-runner $(OSRUNNER_BIN)

update:
	cd src/bootloader && cargo update
	cd src/kernel && cargo update 
	cd src/os-runner && cargo update

$(FAT_IMG): $(BOOT_INFO) $(BUILD_DIR) $(KERNEL_FONT)
	@dd if=/dev/zero of=$(FAT_IMG) bs=1M count=16 status=none
	@mkfs.vfat $(FAT_IMG)
	@mmd -i $(FAT_IMG) ::/efi ::/efi/boot ::/boot
	@mcopy -D o -i $(FAT_IMG) $(BOOT_INFO) $(KERNEL_FONT) ::/boot

make-test-kernel: $(BOOTLOADER_BIN) $(FAT_IMG) $(BUILD_DIR) $(ISO_DIR)
	@mcopy -D o -i $(FAT_IMG) $(BUILD_DIR)/kernel.bin ::/boot 
	@mcopy -D o -i $(FAT_IMG) $(BOOTLOADER_BIN) ::/efi/boot
	@mcopy -D o -i $(FAT_IMG) test_bootinfo.toml $(KERNEL_FONT) ::/boot
	@mmove -D o -i $(FAT_IMG) boot/test_bootinfo.toml boot/bootinfo.toml 
	@cp $(FAT_IMG) $(ISO_DIR)
	@xorriso -as mkisofs -quiet -R -f -e fat.img -no-emul-boot -o $(BUILD_DIR)/os.iso $(ISO_DIR) > /dev/null

$(KERNEL_BIN): $(KERNEL_SOURCES) $(BUILD_DIR)
	cd src/kernel && cargo build $(if $(RELEASE),--release,)
	cp src/kernel/target/x86_64/$(if $(RELEASE),release,debug)/$(NAME) $(KERNEL_BIN)

$(BOOTLOADER_BIN): $(BOOTLOADER_SOURCES) $(BUILD_DIR)
	cd src/bootloader && cargo build $(if $(RELEASE),--release,) 
	cp src/bootloader/target/x86_64-unknown-uefi/$(if $(RELEASE),release,debug)/$(NAME)-bootloader.efi $(BOOTLOADER_BIN)

debug: force_rebuild $(BOOTLOADER_BIN) $(KERNEL_BIN) $(FAT_IMG) $(ISO_DIR)
	mcopy -D o -i $(FAT_IMG) $(KERNEL_BIN) ::/boot 
	mcopy -D o -i $(FAT_IMG) $(BOOTLOADER_BIN) ::/efi/boot
	cp $(FAT_IMG) $(ISO_DIR)
	xorriso -as mkisofs -R -f -e fat.img -no-emul-boot -o $(BUILD_DIR)/os.iso $(ISO_DIR)

release: force_rebuild $(BOOTLOADER_BIN) $(KERNEL_BIN) $(FAT_IMG) $(ISO_DIR)
	mcopy -D o -i $(FAT_IMG) $(KERNEL_BIN) ::/boot 
	mcopy -D o -i $(FAT_IMG) $(BOOTLOADER_BIN) ::/efi/boot
	cp $(FAT_IMG) $(ISO_DIR)
	xorriso -as mkisofs -R -f -e fat.img -no-emul-boot -o $(BUILD_DIR)/os.iso $(ISO_DIR)

test: $(OSRUNNER_BIN)
	cd src/kernel && cargo test $(RUN_ARGS)

clean:
	cd src/common && cargo clean
	cd src/bootloader && cargo clean
	cd src/kernel && cargo clean
	cd src/os-runner && cargo clean
	rm -rf $(BUILD_DIR)
