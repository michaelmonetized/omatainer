/* SPDX-License-Identifier: GPL-2.0-only */
#ifndef OMATAINER_NS7_AUDIO_H
#define OMATAINER_NS7_AUDIO_H
#include "ns7_wire.h"
#ifndef __KERNEL__
typedef uint32_t u32;
#endif

/**
 * ns7_audio_encode - Convert one signed, left-aligned sample to USB PCM.
 * @sample: S32_LE sample bytes
 * @wire: writable three-byte output
 * Return: no value; the upper 24 bits are written in little-endian order.
 */
static inline void ns7_audio_encode(const u8 *sample, u8 *wire)
{
	wire[0] = sample[1];
	wire[1] = sample[2];
	wire[2] = sample[3];
}

/**
 * ns7_audio_decode - Decode one stereo frame from the capture bit planes.
 * @wire: 64 bytes, with left bits at 0..23 and right bits at 32..55
 * @samples: writable eight-byte S32_LE stereo frame
 * Return: no value; reserved bits and the other DMA lanes are ignored.
 */
static inline void ns7_audio_decode(const u8 *wire, u8 *samples)
{
	unsigned int channel, bit, byte;
	for (channel = 0; channel < 2; channel++) {
		u32 value = 0;
		for (bit = 0; bit < 24; bit++)
			value = (value << 1) | (wire[channel * 32 + bit] & 1);
		value <<= 8;
		for (byte = 0; byte < 4; byte++)
			samples[channel * 4 + byte] = value >> (byte * 8);
	}
}

/**
 * ns7_audio_frames - Distribute a millisecond of samples over eight USB slots.
 * @frames: number of sample frames in the millisecond
 * @slot: microframe index, 0..7
 * Return: the slot's frame count, with all eight counts summing to frames.
 */
static inline unsigned int ns7_audio_frames(unsigned int frames,
					    unsigned int slot)
{
	return frames * (slot + 1) / 8 - frames * slot / 8;
}
#endif
