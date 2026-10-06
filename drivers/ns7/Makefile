obj-m += snd_ns7.o
KDIR ?= /lib/modules/$(shell uname -r)/build
BUILD ?= $(CURDIR)/../../target/ns7-driver

.PHONY: all clean test
all:
	mkdir -p $(BUILD)
	cp snd_ns7.c ns7_wire.h ns7_audio.h $(BUILD)/
	printf '%s\n' 'obj-m += snd_ns7.o' > $(BUILD)/Makefile
	$(MAKE) -C $(KDIR) M=$(abspath $(BUILD)) modules
clean:
	$(MAKE) -C $(KDIR) M=$(abspath $(BUILD)) clean

test:
	mkdir -p $(BUILD)
	$(CC) -std=c11 -Wall -Wextra -Werror -fsanitize=undefined,bounds wire-test.c -o $(BUILD)/wire-test
	$(BUILD)/wire-test
