/* SPDX-License-Identifier: GPL-2.0-only */
#ifndef OMATAINER_NS7_WIRE_H
#define OMATAINER_NS7_WIRE_H
#ifdef __KERNEL__
#include <linux/types.h>
#else
#include <stdint.h>
typedef uint8_t u8;
#endif

#define NS7_PACKET 42
#define NS7_PAYLOAD 39

/**
 * ns7_packet_init - Prepare a MIDI transfer with the original driver defaults.
 * @packet: writable buffer of NS7_PACKET bytes
 * Return: no value; the buffer contains idle MIDI and the default control byte.
 */
static inline void ns7_packet_init(u8 *packet)
{
	unsigned int i;
	for (i = 0; i < NS7_PACKET; i++)
		packet[i] = 0xfd;
	packet[NS7_PACKET - 1] = 0xe0;
}

/**
 * ns7_packet_read - Extract MIDI bytes from one completed USB input transfer.
 * @packet: received bytes
 * @length: actual transfer length, at most NS7_PACKET
 * @output: writable buffer of NS7_PACKET bytes
 * Return: number of MIDI bytes; idle and trailing control bytes are excluded.
 */
static inline unsigned int ns7_packet_read(const u8 *packet,
					   unsigned int length, u8 *output)
{
	unsigned int i, count = 0;
	if (length > NS7_PACKET)
		return 0;
	if (length == NS7_PACKET)
		length--;
	for (i = 0; i < length; i++)
		if (packet[i] != 0xfd)
			output[count++] = packet[i];
	return count;
}
#endif
