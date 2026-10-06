/* SPDX-License-Identifier: GPL-2.0-only */
#include "ns7_audio.h"
#include "ns7_wire.h"
#include <assert.h>
#include <string.h>

int main(void)
{
	u8 packet[NS7_PACKET], received[NS7_PACKET];
	ns7_packet_init(packet);
	assert(packet[41] == 0xe0);
	assert(ns7_packet_read(packet, NS7_PACKET, received) == 0);
	memcpy(packet, (u8[]){0x90, 0x11, 0x7f, 0xb0, 4, 64}, 6);
	assert(ns7_packet_read(packet, NS7_PACKET, received) == 6);
	assert(!memcmp(received, (u8[]){0x90, 0x11, 0x7f, 0xb0, 4, 64}, 6));
	assert(ns7_packet_read(packet, 3, received) == 3);
	assert(ns7_packet_read(packet, 43, received) == 0);
	memset(packet, 0x7f, NS7_PAYLOAD);
	assert(packet[39] == 0xfd && packet[40] == 0xfd && packet[41] == 0xe0);
	assert(ns7_packet_read(packet, NS7_PACKET, received) == NS7_PAYLOAD);
	u8 sample[] = {0xff, 0x56, 0x34, 0x12}, encoded[3];
	ns7_audio_encode(sample, encoded);
	assert(!memcmp(encoded, (u8[]){0x56, 0x34, 0x12}, 3));
	u8 capture[64], decoded[8];
	memset(capture, 0xfe, sizeof(capture));
	u32 left = 0x812345, right = 0x7abcde;
	for (unsigned int bit = 0; bit < 24; bit++) {
		capture[bit] |= (left >> (23 - bit)) & 1;
		capture[32 + bit] |= (right >> (23 - bit)) & 1;
	}
	ns7_audio_decode(capture, decoded);
	assert(!memcmp(decoded,
		       (u8[]){0, 0x45, 0x23, 0x81, 0, 0xde, 0xbc, 0x7a}, 8));
	for (unsigned int frames = 44; frames <= 45; frames++) {
		unsigned int total = 0;
		for (unsigned int slot = 0; slot < 8; slot++) {
			unsigned int n = ns7_audio_frames(frames, slot);
			assert(n >= 5 && n <= 6);
			total += n;
		}
		assert(total == frames);
	}
	return 0;
}
