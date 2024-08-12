ifeq (test,$(firstword $(MAKECMDGOALS)))
  RUN_ARGS := $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS))
  $(eval $(RUN_ARGS):;@:)
endif

.PHONY: debug release clean fat run maker-no-kernel os-runner test-run test disk update
.DEFAULT_GOAL := debug

NAME := nothingos

directory:
	mkdir -p build
	mkdir -p build/iso

disk:
	qemu-img create -f qcow2 disk.img 1G

run: 
	qemu-system-x86_64 -cdrom build/os.iso -m 1G -bios OVMF.fd -drive id=disk,file=disk.img,if=none,format=qcow2 -device ahci,id=ahci -device ide-hd,drive=disk,bus=ahci.0 -boot d -machine kernel_irqchip=split -no-reboot -enable-kvm -cpu host,+rdrand -display gtk 

test-run:
	qemu-system-x86_64 -cdrom build/os.iso -m 1G -bios OVMF.fd -serial stdio -drive id=disk,file=disk.img,if=none,format=qcow2 -device isa-debug-exit,iobase=0xf4,iosize=0x04 -device ahci,id=ahci -device ide-hd,drive=disk,bus=ahci.0 -boot d -machine kernel_irqchip=split -no-reboot -enable-kvm -cpu host,+rdrand -display none

os-runner: directory
	@cd src/os-runner && cargo build --release --quiet
	@cp src/os-runner/target/release/os-runner build/os-runner

update:
	cd src/bootloader && cargo update
	cd src/kernel && cargo update 
	cd src/os-runner && cargo update

maker-no-kernel: fat
	@cd src/bootloader && cargo build --target x86_64-unknown-uefi && cp target/x86_64-unknown-uefi/debug/$(NAME).efi target/x86_64-unknown-uefi/debug/bootx64.efi
	@mcopy -i build/fat.img src/bootloader/target/x86_64-unknown-uefi/debug/bootx64.efi ::/efi/boot
	@mcopy -i build/fat.img build/kernel.bin ::/boot 
	@mcopy -i build/fat.img bootinfo.toml ::/boot
	@mcopy -i build/fat.img kernel-font.ttf ::/boot
	@cp build/fat.img build/iso
	@xorriso -as mkisofs -quiet -R -f -e fat.img -no-emul-boot -o build/os.iso build/iso > /dev/null

debug-kernel: fat
	cd src/kernel && cargo build
	cd src/bootloader && cargo build --target x86_64-unknown-uefi && cp target/x86_64-unknown-uefi/debug/$(NAME).efi target/x86_64-unknown-uefi/debug/bootx64.efi
	mcopy -i build/fat.img src/bootloader/target/x86_64-unknown-uefi/debug/bootx64.efi ::/efi/boot
	cp src/kernel/target/x86_64/debug/${NAME} build/kernel.bin

test: os-runner
	cd src/kernel && cargo test $(RUN_ARGS)

release-kernel: fat
	cd src/kernel && cargo build --release
	cd src/bootloader && cargo build --release --target x86_64-unknown-uefi && cp target/x86_64-unknown-uefi/release/$(NAME).efi target/x86_64-unknown-uefi/release/bootx64.efi
	mcopy -i build/fat.img src/bootloader/target/x86_64-unknown-uefi/release/bootx64.efi ::/efi/boot
	cp src/kernel/target/x86_64/release/${NAME} build/kernel.bin

debug: directory debug-kernel
	mcopy -i build/fat.img build/kernel.bin ::/boot 
	mcopy -i build/fat.img bootinfo.toml ::/boot
	mcopy -i build/fat.img kernel-font.ttf ::/boot
	cp build/fat.img build/iso
	xorriso -as mkisofs -R -f -e fat.img -no-emul-boot -o build/os.iso build/iso

release: directory release-kernel
	mcopy -i build/fat.img build/kernel.bin ::/boot 
	mcopy -i build/fat.img bootinfo.toml ::/boot
	mcopy -i build/fat.img kernel-font.ttf ::/boot
	cp build/fat.img build/iso
	xorriso -as mkisofs -R -f -e fat.img -no-emul-boot -o build/os.iso build/iso

fat:
	@dd if=/dev/zero of=build/fat.img bs=1M count=16 status=none
	@mkfs.vfat build/fat.img
	@mmd -i build/fat.img ::/efi
	@mmd -i build/fat.img ::/efi/boot
	@mmd -i build/fat.img ::/boot

clean:
	cd src/bootloader && cargo clean
	cd src/kernel && cargo clean
	cd src/os-runner && cargo clean
	rm -rf build
	rm -rf disk.img
