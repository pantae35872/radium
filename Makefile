ifeq (test,$(firstword $(MAKECMDGOALS)))
  RUN_ARGS := $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS))
  $(eval $(RUN_ARGS):;@:)
endif
ifeq (make-test-kernel,$(firstword $(MAKECMDGOALS)))
	STILL_TESTING := 1
else 
	STILL_TESTING := 0
endif
ifeq (release,$(firstword $(MAKECMDGOALS))) 
	RELEASE := 1
  BUILD_MODE := release
else 
  BUILD_MODE := debug
endif


.PHONY: debug release clean run make-test-kernel test-run test update dbg-run force_rebuild
.DEFAULT_GOAL := debug

NAME := radium
BUILD_DIR := build
ISO_DIR := $(BUILD_DIR)/iso
ISO_FILE := $(BUILD_DIR)/os.iso
FAT_IMG := $(BUILD_DIR)/fat.img
DISK_FILE := disk.img

# Dependency files
KERNEL_DEPS := $(wildcard $(BUILD_DIR)/x86_64/$(BUILD_MODE)/*.d)
BOOTLOADER_DEPS := $(wildcard $(BUILD_DIR)/x86_64-unknown-uefi/$(BUILD_MODE)/*.d)
OSRUNNER_DEPS := $(wildcard $(BUILD_DIR)/release/*.d)

KERNEL_OPTS_DEPS := src/kernel/src/boot/boot.asm

# Binaries
KERNEL_BIN := $(abspath $(BUILD_DIR)/x86_64/$(BUILD_MODE)/$(NAME))
OSRUNNER_BIN := $(abspath $(BUILD_DIR)/release/os-runner)
BOOTLOADER_BIN := $(abspath $(BUILD_DIR)/x86_64-unknown-uefi/$(BUILD_MODE)/$(NAME)-bootloader.efi)
BUILD_MODE_FILE := $(BUILD_DIR)/.build_mode
BOOT_INFO := bootinfo.toml
KERNEL_FONT := kernel-font.ttf

OVMF := OVMF.fd

ifeq ($(BUILD_MODE), $(shell cat $(BUILD_MODE_FILE) 2>/dev/null))
    BUILD_MODE_CHANGED := 0
else
    BUILD_MODE_CHANGED := 1
endif

-include $(KERNEL_DEPS)
-include $(BOOTLOADER_DEPS)
-include $(OSRUNNER_DEPS)

QEMU_FLAGS := -m 1G -bios OVMF.fd -serial stdio \
	-drive id=disk,file=$(DISK_FILE),if=none,format=qcow2 -device ahci,id=ahci \
	-device ide-hd,drive=disk,bus=ahci.0 -boot d -machine kernel_irqchip=split \
	-no-reboot

KVM_FLAGS := -enable-kvm -cpu host,+rdrand,+sse,+mmx

$(BUILD_DIR):
	mkdir $(BUILD_DIR)

$(ISO_DIR):
	mkdir $(ISO_DIR)


ifneq ($(STILL_TESTING),1)
ifeq ($(BUILD_MODE_CHANGED),1)
force_rebuild:

$(KERNEL_BIN): force_rebuild
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
	qemu-system-x86_64 $(QEMU_FLAGS) -display sdl -cdrom $(BUILD_DIR)/os.iso 

test-run: $(DISK_FILE) $(OVMF)
	qemu-system-x86_64 $(QEMU_FLAGS) $(KVM_FLAGS) -cdrom $(BUILD_DIR)/test.iso -device isa-debug-exit,iobase=0xf4,iosize=0x04 -display none 

$(OSRUNNER_BIN): $(BUILD_DIR) 
	cd src/os-runner && cargo build --release --quiet

$(BUILD_MODE_FILE): $(BUILD_DIR) force_rebuild
	@echo $(BUILD_MODE) > $(BUILD_MODE_FILE)

make-test-kernel: $(FAT_IMG) $(ISO_DIR)
	@echo test > $(BUILD_MODE_FILE)
	mcopy -D o -i $(FAT_IMG) $(BUILD_DIR)/kernel.bin ::/boot
	mcopy -D o -i $(FAT_IMG) test_bootinfo.toml $(KERNEL_FONT) ::/boot
	mmove -D o -i $(FAT_IMG) boot/test_bootinfo.toml boot/bootinfo.toml
	cp $(FAT_IMG) $(ISO_DIR)
	xorriso -as mkisofs -quiet -R -f -e fat.img -no-emul-boot -o $(BUILD_DIR)/test.iso $(ISO_DIR) > /dev/null

$(KERNEL_FONT):
	wget https://www.1001fonts.com/download/font/open-sans.regular.ttf
	mv open-sans.regular.ttf kernel-font.ttf

$(KERNEL_BIN): $(KERNEL_OPTS_DEPS)
ifneq ($(STILL_TESTING),1)
	cd src/kernel && cargo build $(if $(RELEASE),--release,)
	cp $(KERNEL_BIN) $(BUILD_DIR)/kernel.bin
endif

$(BOOTLOADER_BIN):
ifneq ($(STILL_TESTING),1)
	cd src/bootloader && cargo build $(if $(RELEASE),--release,) 
	cp $(BOOTLOADER_BIN) $(BUILD_DIR)/BOOTX64.EFI
endif

$(FAT_IMG): $(BOOT_INFO) $(BUILD_DIR) $(KERNEL_FONT) $(KERNEL_BIN) $(BOOTLOADER_BIN) 	
ifneq ($(STILL_TESTING),1)
	dd if=/dev/zero of=$(FAT_IMG) bs=1M count=16 status=none
	mkfs.vfat $(FAT_IMG)
	mmd -i $(FAT_IMG) ::/EFI ::/EFI/BOOT ::/boot
	mcopy -D o -i $(FAT_IMG) $(BOOT_INFO) $(KERNEL_FONT) ::/boot
	mcopy -D o -i $(FAT_IMG) $(BUILD_DIR)/kernel.bin ::/boot 
	mcopy -D o -i $(FAT_IMG) $(BUILD_DIR)/BOOTX64.EFI ::/EFI/BOOT
endif

$(ISO_FILE): $(FAT_IMG) $(ISO_DIR)
	cp $(FAT_IMG) $(ISO_DIR)
	xorriso -as mkisofs -R -f -e fat.img -no-emul-boot -o $(BUILD_DIR)/os.iso $(ISO_DIR)

release: $(BUILD_MODE_FILE) $(OVMF) $(ISO_FILE)
debug: $(BUILD_MODE_FILE) $(OVMF) $(ISO_FILE) 

test: $(OSRUNNER_BIN)
	cd src/kernel && cargo test $(RUN_ARGS)

clean:
	rm -rf $(BUILD_DIR)
	rm -rf $(DISK_FILE)
	rm -rf $(KERNEL_FONT)
