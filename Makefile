ifeq (test,$(firstword $(MAKECMDGOALS)))
  RUN_ARGS := $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS))
  $(eval $(RUN_ARGS):;@:)
endif

.PHONY: debug release clean fat run maker-no-kernel os-runner test-run test disk update
.DEFAULT_GOAL := debug

NAME := nothingos
BUILD_DIR := build
ISO_DIR := $(BUILD_DIR)/iso
FAT_IMG := $(BUILD_DIR)/fat.img
KERNEL_BIN := $(BUILD_DIR)/kernel.bin

directory:
	mkdir -p $(ISO_DIR)

disk:
	qemu-img create -f qcow2 disk.img 1G

run: 
	qemu-system-x86_64 -cdrom $(BUILD_DIR)/os.iso -m 1G -bios OVMF.fd \
	-drive id=disk,file=disk.img,if=none,format=qcow2 -device ahci,id=ahci \
	-device ide-hd,drive=disk,bus=ahci.0 -boot d -machine kernel_irqchip=split \
	-no-reboot -enable-kvm -cpu host,+rdrand -display gtk #-d trace:ahci*

test-run:
	qemu-system-x86_64 -cdrom $(BUILD_DIR)/os.iso -m 1G -bios OVMF.fd -serial stdio \
	-drive id=disk,file=disk.img,if=none,format=qcow2 -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
	-device ahci,id=ahci -device ide-hd,drive=disk,bus=ahci.0 -boot d -machine kernel_irqchip=split \
	-no-reboot -enable-kvm -cpu host,+rdrand -display none 

os-runner: directory
	@cd src/os-runner && cargo build --release --quiet
	@cp src/os-runner/target/release/os-runner $(BUILD_DIR)/os-runner

update:
	cd src/bootloader && cargo update
	cd src/kernel && cargo update 
	cd src/os-runner && cargo update

maker-no-kernel: fat
	$(call build_bootloader)
	@mcopy -i $(FAT_IMG) $(BUILD_DIR)/kernel.bin ::/boot 
	@mcopy -i $(FAT_IMG) bootinfo.toml kernel-font.ttf ::/boot
	@cp $(FAT_IMG) $(ISO_DIR)
	@xorriso -as mkisofs -quiet -R -f -e fat.img -no-emul-boot -o $(BUILD_DIR)/os.iso $(ISO_DIR) > /dev/null

define build_bootloader
	cd src/bootloader && cargo build $(if $(1),--release,) --target x86_64-unknown-uefi && \
	cp target/x86_64-unknown-uefi/$(if $(1),release,debug)/$(NAME).efi target/x86_64-unknown-uefi/$(if $(1),release,debug)/bootx64.efi
	mcopy -i $(FAT_IMG) src/bootloader/target/x86_64-unknown-uefi/$(if $(1),release,debug)/bootx64.efi ::/efi/boot
endef

debug-kernel: fat
	cd src/kernel && cargo build
	$(call build_bootloader)
	cp src/kernel/target/x86_64/debug/$(NAME) $(KERNEL_BIN)

release-kernel: fat
	cd src/kernel && cargo build --release
	$(call build_bootloader,release)
	cp src/kernel/target/x86_64/release/$(NAME) $(KERNEL_BIN)

debug: directory debug-kernel
	$(call prepare_iso)

release: directory release-kernel
	$(call prepare_iso)

fat:
	@dd if=/dev/zero of=$(FAT_IMG) bs=1M count=16 status=none
	@mkfs.vfat $(FAT_IMG)
	@mmd -i $(FAT_IMG) ::/efi ::/efi/boot ::/boot

define prepare_iso
	mcopy -i $(FAT_IMG) $(KERNEL_BIN) ::/boot 
	mcopy -i $(FAT_IMG) bootinfo.toml kernel-font.ttf ::/boot
	cp $(FAT_IMG) $(ISO_DIR)
	xorriso -as mkisofs -R -f -e fat.img -no-emul-boot -o $(BUILD_DIR)/os.iso $(ISO_DIR)
endef

test: os-runner
	cd src/kernel && cargo test $(RUN_ARGS)

clean:
	cd src/bootloader && cargo clean
	cd src/kernel && cargo clean
	cd src/os-runner && cargo clean
	rm -rf $(BUILD_DIR) disk.img
