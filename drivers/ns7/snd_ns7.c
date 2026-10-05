// SPDX-License-Identifier: GPL-2.0-only
#include "ns7_audio.h"
#include "ns7_wire.h"
#include <linux/module.h>
#include <linux/delay.h>
#include <linux/slab.h>
#include <linux/usb.h>
#include <sound/core.h>
#include <sound/pcm.h>
#include <sound/pcm_params.h>
#include <sound/rawmidi.h>

#define NS7_READS 16
#define NS7_AUDIO_URBS 4
#define NS7_FEEDBACK_URBS 4
#define NS7_FEEDBACK_QUEUE 128
static struct usb_driver ns7_driver;
struct ns7;

struct ns7_audio_urb {
	struct ns7 *chip;
	struct urb *urb;
	unsigned int frames;
	unsigned int generation;
};

struct ns7_pcm_state {
	struct snd_pcm_substream *stream;
	snd_pcm_uframes_t position;
	snd_pcm_uframes_t queued;
	unsigned int period;
	unsigned int generation;
	bool prepared;
	bool running;
};

struct ns7 {
	struct usb_device *usb;
	struct snd_card *card;
	struct usb_interface *auxiliary;
	spinlock_t lock;
	struct snd_rawmidi_substream *input;
	struct snd_rawmidi_substream *output;
	struct urb *reads[NS7_READS];
	struct urb *write;
	u8 *write_buffer;
	bool dead;
	bool output_enabled;
	bool writing;
	int pending;
	u8 saved_status;
	bool configured;
	bool configuring;
	atomic64_t received;
	atomic64_t sent;
	atomic64_t errors;
	struct mutex audio_mutex;
	struct ns7_pcm_state pcm[2];
	struct ns7_audio_urb playback[NS7_AUDIO_URBS];
	struct ns7_audio_urb capture[NS7_AUDIO_URBS];
	struct urb *feedback[NS7_FEEDBACK_URBS];
	bool audio_active;
	u8 feedback_queue[NS7_FEEDBACK_QUEUE];
	unsigned int feedback_head;
	unsigned int feedback_tail;
	unsigned int nominal_phase;
	atomic64_t playback_frames;
	atomic64_t capture_frames;
	atomic64_t audio_errors;
	atomic64_t feedback_frames;
	atomic64_t feedback_idle;
};

static void ns7_send_locked(struct ns7 *chip);

static void ns7_midi_pause(struct ns7 *chip)
{
	unsigned long flags;
	spin_lock_irqsave(&chip->lock, flags);
	chip->configuring = true;
	spin_unlock_irqrestore(&chip->lock, flags);
	for (int i = 0; i < NS7_READS; i++)
		usb_kill_urb(chip->reads[i]);
	usb_kill_urb(chip->write);
}

static int ns7_midi_resume(struct ns7 *chip)
{
	unsigned long flags;
	int result = 0;
	spin_lock_irqsave(&chip->lock, flags);
	chip->configuring = false;
	if (chip->dead) {
		spin_unlock_irqrestore(&chip->lock, flags);
		return -ENODEV;
	}
	spin_unlock_irqrestore(&chip->lock, flags);
	for (int i = 0; i < NS7_READS; i++) {
		result = usb_submit_urb(chip->reads[i], GFP_KERNEL);
		if (result) {
			atomic64_inc(&chip->errors);
			ns7_midi_pause(chip);
			return result;
		}
	}
	spin_lock_irqsave(&chip->lock, flags);
	ns7_send_locked(chip);
	spin_unlock_irqrestore(&chip->lock, flags);
	return result;
}

static bool ns7_cancelled(struct urb *urb)
{
	return urb->status == -ENOENT || urb->status == -ECONNRESET ||
	       urb->status == -ESHUTDOWN;
}

static struct snd_pcm_substream *ns7_pcm_advance(struct ns7_pcm_state *state,
						 unsigned int frames)
{
	struct snd_pcm_runtime *runtime = state->stream->runtime;
	state->position = (state->position + frames) % runtime->buffer_size;
	state->period += frames;
	if (state->period < runtime->period_size)
		return NULL;
	state->period %= runtime->period_size;
	return state->stream;
}

static void ns7_playback_fill(struct ns7_audio_urb *transfer)
{
	struct ns7 *chip = transfer->chip;
	struct urb *urb = transfer->urb;
	struct ns7_pcm_state *state = &chip->pcm[SNDRV_PCM_STREAM_PLAYBACK];
	u8 *wire = urb->transfer_buffer;
	unsigned int frames, slot, frame, channel, offset = 0;
	chip->nominal_phase += 44100;
	frames = chip->nominal_phase / 1000;
	chip->nominal_phase %= 1000;
	if (chip->feedback_tail != chip->feedback_head) {
		frames = chip->feedback_queue[chip->feedback_tail];
		chip->feedback_tail =
		    (chip->feedback_tail + 1) % NS7_FEEDBACK_QUEUE;
	}
	memset(wire, 0, 156 * 8);
	transfer->frames = state->running ? frames : 0;
	transfer->generation = state->generation;
	for (slot = 0; slot < 8; slot++) {
		unsigned int count = ns7_audio_frames(frames, slot);
		urb->iso_frame_desc[slot].offset = offset;
		urb->iso_frame_desc[slot].length = count * 12;
		if (state->running) {
			struct snd_pcm_runtime *runtime =
			    state->stream->runtime;
			for (frame = 0; frame < count; frame++) {
				u8 *sample =
				    runtime->dma_area + state->queued * 16;
				for (channel = 0; channel < 4; channel++)
					ns7_audio_encode(sample + channel * 4,
							 wire + offset +
							     frame * 12 +
							     channel * 3);
				state->queued =
				    (state->queued + 1) % runtime->buffer_size;
			}
		}
		offset += count * 12;
	}
	urb->transfer_buffer_length = offset;
}

static void ns7_playback_complete(struct urb *urb)
{
	struct ns7_audio_urb *transfer = urb->context;
	struct ns7 *chip = transfer->chip;
	struct snd_pcm_substream *notify = NULL;
	struct ns7_pcm_state *state = &chip->pcm[SNDRV_PCM_STREAM_PLAYBACK];
	unsigned long flags;
	unsigned int slot, frames = 0;
	int result;
	if (ns7_cancelled(urb))
		return;
	spin_lock_irqsave(&chip->lock, flags);
	if (!chip->audio_active || chip->dead)
		goto unlock;
	if (urb->status)
		atomic64_inc(&chip->audio_errors);
	else {
		for (slot = 0; slot < 8; slot++) {
			if (urb->iso_frame_desc[slot].status)
				atomic64_inc(&chip->audio_errors);
			else
				frames +=
				    urb->iso_frame_desc[slot].actual_length /
				    12;
		}
		atomic64_add(frames, &chip->playback_frames);
		if (state->running && transfer->frames &&
		    state->generation == transfer->generation)
			notify = ns7_pcm_advance(state, transfer->frames);
	}
	ns7_playback_fill(transfer);
	result = usb_submit_urb(urb, GFP_ATOMIC);
	if (result)
		atomic64_inc(&chip->audio_errors);
unlock:
	spin_unlock_irqrestore(&chip->lock, flags);
	if (notify)
		snd_pcm_period_elapsed(notify);
}

static void ns7_capture_complete(struct urb *urb)
{
	struct ns7_audio_urb *transfer = urb->context;
	struct ns7 *chip = transfer->chip;
	struct ns7_pcm_state *state = &chip->pcm[SNDRV_PCM_STREAM_CAPTURE];
	struct snd_pcm_substream *notify = NULL;
	unsigned long flags;
	unsigned int offset, frames;
	int result;
	if (ns7_cancelled(urb))
		return;
	spin_lock_irqsave(&chip->lock, flags);
	if (!chip->audio_active || chip->dead)
		goto unlock;
	if (urb->status || urb->actual_length % 64) {
		atomic64_inc(&chip->audio_errors);
	} else {
		frames = urb->actual_length / 64;
		atomic64_add(frames, &chip->capture_frames);
		if (state->running) {
			struct snd_pcm_runtime *runtime =
			    state->stream->runtime;
			for (offset = 0; offset < urb->actual_length;
			     offset += 64) {
				ns7_audio_decode(urb->transfer_buffer + offset,
						 runtime->dma_area +
						     state->position * 8);
				if (ns7_pcm_advance(state, 1))
					notify = state->stream;
			}
		}
	}
	result = usb_submit_urb(urb, GFP_ATOMIC);
	if (result)
		atomic64_inc(&chip->audio_errors);
unlock:
	spin_unlock_irqrestore(&chip->lock, flags);
	if (notify)
		snd_pcm_period_elapsed(notify);
}

static void ns7_feedback_complete(struct urb *urb)
{
	struct ns7 *chip = urb->context;
	unsigned long flags;
	unsigned int next;
	u8 *wire = urb->transfer_buffer;
	int result;
	if (ns7_cancelled(urb))
		return;
	spin_lock_irqsave(&chip->lock, flags);
	if (!chip->audio_active || chip->dead)
		goto unlock;
	if (!urb->status && !urb->iso_frame_desc[0].status &&
	    urb->iso_frame_desc[0].actual_length == 3 && wire[0] >= 43 &&
	    wire[0] <= 46) {
		next = (chip->feedback_head + 1) % NS7_FEEDBACK_QUEUE;
		if (next != chip->feedback_tail) {
			chip->feedback_queue[chip->feedback_head] = wire[0];
			chip->feedback_head = next;
		}
		atomic64_inc(&chip->feedback_frames);
	} else if (!urb->status && !urb->iso_frame_desc[0].status &&
		   (!urb->iso_frame_desc[0].actual_length ||
		    (urb->iso_frame_desc[0].actual_length == 3 && !wire[0]))) {
		atomic64_inc(&chip->feedback_idle);
	} else {
		atomic64_inc(&chip->audio_errors);
	}
	result = usb_submit_urb(urb, GFP_ATOMIC);
	if (result)
		atomic64_inc(&chip->audio_errors);
unlock:
	spin_unlock_irqrestore(&chip->lock, flags);
}

static void ns7_audio_stop(struct ns7 *chip)
{
	unsigned long flags;
	int i;
	spin_lock_irqsave(&chip->lock, flags);
	chip->audio_active = false;
	spin_unlock_irqrestore(&chip->lock, flags);
	for (i = 0; i < NS7_AUDIO_URBS; i++) {
		usb_kill_urb(chip->playback[i].urb);
		usb_kill_urb(chip->capture[i].urb);
	}
	for (i = 0; i < NS7_FEEDBACK_URBS; i++)
		usb_kill_urb(chip->feedback[i]);
}

static int ns7_audio_start(struct ns7 *chip)
{
	unsigned long flags;
	u8 *rate = kmalloc(3, GFP_KERNEL);
	s64 clock = atomic64_read(&chip->feedback_frames);
	int i, result;
	if (!rate)
		return -ENOMEM;
	ns7_midi_pause(chip);
	rate[0] = 0x44;
	rate[1] = 0xac;
	rate[2] = 0;
	result = usb_control_msg(chip->usb, usb_sndctrlpipe(chip->usb, 0), 1,
				 0x22, 0x100, 0x81, rate, 3, 1000);
	if (result == 3)
		result = usb_control_msg(chip->usb, usb_sndctrlpipe(chip->usb, 0), 1,
				 0x22, 0x100, 0x02, rate, 3, 1000);
	if (result == 3)
		result = usb_control_msg(chip->usb, usb_sndctrlpipe(chip->usb, 0), 1,
					 0x22, 0x100, 0x86, rate, 3, 1000);
	if (result != 3) {
		kfree(rate);
		ns7_midi_resume(chip);
		return result < 0 ? result : -EIO;
	}
	result = usb_control_msg(chip->usb, usb_rcvctrlpipe(chip->usb, 0), 0x81,
				 0xa2, 0x100, 0x86, rate, 3, 1000);
	if (result != 3 || rate[0] != 0x44 || rate[1] != 0xac || rate[2]) {
		kfree(rate);
		ns7_midi_resume(chip);
		return result < 0 ? result : -EIO;
	}
	msleep(50);
	result = usb_control_msg(chip->usb, usb_rcvctrlpipe(chip->usb, 0), 0x49,
				 0xc0, 0, 0, rate, 1, 1000);
	if (result == 1)
		result = usb_control_msg(chip->usb, usb_sndctrlpipe(chip->usb, 0),
					 0x49, 0x40, (u16)(s16)(s8)(rate[0] | 0x30),
					 0, NULL, 0, 1000);
	else if (result >= 0)
		result = -EIO;
	kfree(rate);
	if (result) {
		ns7_midi_resume(chip);
		return result;
	}
	result = ns7_midi_resume(chip);
	if (result)
		return result;
	spin_lock_irqsave(&chip->lock, flags);
	chip->audio_active = true;
	chip->nominal_phase = 0;
	chip->feedback_head = chip->feedback_tail = 0;
	spin_unlock_irqrestore(&chip->lock, flags);
	for (i = 0; i < NS7_FEEDBACK_URBS; i++) {
		result = usb_submit_urb(chip->feedback[i], GFP_KERNEL);
		if (result)
			goto fail;
	}
	for (i = 0; i < NS7_AUDIO_URBS; i++) {
		spin_lock_irqsave(&chip->lock, flags);
		ns7_playback_fill(&chip->playback[i]);
		spin_unlock_irqrestore(&chip->lock, flags);
		result = usb_submit_urb(chip->playback[i].urb, GFP_KERNEL);
		if (result)
			goto fail;
		result = usb_submit_urb(chip->capture[i].urb, GFP_KERNEL);
		if (result)
			goto fail;
	}
	for (i = 0; i < 50; i++) {
		if (atomic64_read(&chip->feedback_frames) > clock)
			return 0;
		if (READ_ONCE(chip->dead)) {
			result = -ENODEV;
			goto fail;
		}
		msleep(10);
	}
	dev_err(&chip->usb->dev,
		"NS7 audio clock is idle; reset USB and reopen the output\n");
	result = -ETIMEDOUT;
fail:
	ns7_audio_stop(chip);
	return result;
}

static const struct snd_pcm_hardware ns7_pcm_hardware = {
    .info = SNDRV_PCM_INFO_MMAP | SNDRV_PCM_INFO_INTERLEAVED |
	    SNDRV_PCM_INFO_MMAP_VALID | SNDRV_PCM_INFO_BATCH,
    .formats = SNDRV_PCM_FMTBIT_S32_LE,
    .rates = SNDRV_PCM_RATE_44100,
    .rate_min = 44100,
    .rate_max = 44100,
    .buffer_bytes_max = 1024 * 1024,
    .period_bytes_min = 256,
    .period_bytes_max = 128 * 1024,
    .periods_min = 2,
    .periods_max = 1024,
};

static int ns7_pcm_open(struct snd_pcm_substream *stream)
{
	struct ns7 *chip = snd_pcm_substream_chip(stream);
	unsigned int channels =
	    stream->stream == SNDRV_PCM_STREAM_PLAYBACK ? 4 : 2;
	if (READ_ONCE(chip->dead))
		return -ENODEV;
	stream->runtime->hw = ns7_pcm_hardware;
	stream->runtime->hw.channels_min = channels;
	stream->runtime->hw.channels_max = channels;
	return snd_pcm_hw_constraint_minmax(
	    stream->runtime, SNDRV_PCM_HW_PARAM_PERIOD_SIZE, 128, 8192);
}

static int ns7_pcm_hw_free(struct snd_pcm_substream *stream)
{
	struct ns7 *chip = snd_pcm_substream_chip(stream);
	struct ns7_pcm_state *state = &chip->pcm[stream->stream];
	unsigned long flags;
	mutex_lock(&chip->audio_mutex);
	spin_lock_irqsave(&chip->lock, flags);
	state->running = false;
	state->prepared = false;
	state->generation++;
	spin_unlock_irqrestore(&chip->lock, flags);
	if (!chip->pcm[0].prepared && !chip->pcm[1].prepared)
		ns7_audio_stop(chip);
	else {
		for (int i = 0; i < NS7_AUDIO_URBS; i++)
			usb_kill_urb(stream->stream == SNDRV_PCM_STREAM_PLAYBACK
					 ? chip->playback[i].urb
					 : chip->capture[i].urb);
		for (int i = 0; i < NS7_AUDIO_URBS; i++) {
			struct ns7_audio_urb *transfer =
			    stream->stream == SNDRV_PCM_STREAM_PLAYBACK
				? &chip->playback[i]
				: &chip->capture[i];
			if (stream->stream == SNDRV_PCM_STREAM_PLAYBACK) {
				spin_lock_irqsave(&chip->lock, flags);
				ns7_playback_fill(transfer);
				spin_unlock_irqrestore(&chip->lock, flags);
			}
			if (usb_submit_urb(transfer->urb, GFP_KERNEL))
				atomic64_inc(&chip->audio_errors);
		}
	}
	spin_lock_irqsave(&chip->lock, flags);
	state->stream = NULL;
	spin_unlock_irqrestore(&chip->lock, flags);
	mutex_unlock(&chip->audio_mutex);
	return 0;
}

static int ns7_pcm_prepare(struct snd_pcm_substream *stream)
{
	struct ns7 *chip = snd_pcm_substream_chip(stream);
	struct ns7_pcm_state *state = &chip->pcm[stream->stream];
	unsigned long flags;
	int result = 0;
	mutex_lock(&chip->audio_mutex);
	spin_lock_irqsave(&chip->lock, flags);
	if (chip->dead) {
		result = -ENODEV;
	} else {
		state->stream = stream;
		state->running = false;
		state->prepared = true;
		state->generation++;
		state->position = state->queued = 0;
		state->period = 0;
	}
	spin_unlock_irqrestore(&chip->lock, flags);
	if (!result && !chip->audio_active)
		result = ns7_audio_start(chip);
	mutex_unlock(&chip->audio_mutex);
	return result;
}

static int ns7_pcm_trigger(struct snd_pcm_substream *stream, int command)
{
	struct ns7 *chip = snd_pcm_substream_chip(stream);
	struct ns7_pcm_state *state = &chip->pcm[stream->stream];
	unsigned long flags;
	int result = 0;
	spin_lock_irqsave(&chip->lock, flags);
	if (chip->dead) {
		result = -ENODEV;
	} else if (command == SNDRV_PCM_TRIGGER_START) {
		state->running = true;
	} else if (command == SNDRV_PCM_TRIGGER_STOP) {
		state->running = false;
		state->generation++;
	} else {
		result = -EINVAL;
	}
	spin_unlock_irqrestore(&chip->lock, flags);
	return result;
}

static snd_pcm_uframes_t ns7_pcm_pointer(struct snd_pcm_substream *stream)
{
	struct ns7 *chip = snd_pcm_substream_chip(stream);
	unsigned long flags;
	snd_pcm_uframes_t position;
	spin_lock_irqsave(&chip->lock, flags);
	position = chip->pcm[stream->stream].position;
	spin_unlock_irqrestore(&chip->lock, flags);
	return position;
}

static const struct snd_pcm_ops ns7_pcm_ops = {
    .open = ns7_pcm_open,
    .close = ns7_pcm_hw_free,
    .ioctl = snd_pcm_lib_ioctl,
    .hw_free = ns7_pcm_hw_free,
    .prepare = ns7_pcm_prepare,
    .trigger = ns7_pcm_trigger,
    .pointer = ns7_pcm_pointer,
};

static struct urb *ns7_iso_urb(struct ns7 *chip, unsigned int endpoint,
			       unsigned int packets, unsigned int bytes,
			       unsigned int interval, usb_complete_t callback,
			       void *context)
{
	struct urb *urb = usb_alloc_urb(packets, GFP_KERNEL);
	if (!urb)
		return NULL;
	urb->transfer_buffer = kmalloc(bytes * packets, GFP_KERNEL);
	if (!urb->transfer_buffer) {
		usb_free_urb(urb);
		return NULL;
	}
	urb->dev = chip->usb;
	urb->pipe = endpoint & USB_DIR_IN
			? usb_rcvisocpipe(chip->usb, endpoint & 15)
			: usb_sndisocpipe(chip->usb, endpoint);
	urb->transfer_flags = URB_ISO_ASAP;
	urb->transfer_buffer_length = bytes * packets;
	urb->number_of_packets = packets;
	urb->interval = interval;
	urb->context = context;
	urb->complete = callback;
	for (unsigned int i = 0; i < packets; i++) {
		urb->iso_frame_desc[i].offset = i * bytes;
		urb->iso_frame_desc[i].length = bytes;
	}
	return urb;
}

static int ns7_pcm_new(struct ns7 *chip)
{
	struct snd_pcm *pcm;
	int i, result;
	result = snd_pcm_new(chip->card, "Numark NS7 Audio", 0, 1, 1, &pcm);
	if (result)
		return result;
	pcm->private_data = chip;
	strscpy(pcm->name, "Numark NS7 Audio", sizeof(pcm->name));
	snd_pcm_set_ops(pcm, SNDRV_PCM_STREAM_PLAYBACK, &ns7_pcm_ops);
	snd_pcm_set_ops(pcm, SNDRV_PCM_STREAM_CAPTURE, &ns7_pcm_ops);
	result = snd_pcm_set_managed_buffer_all(pcm, SNDRV_DMA_TYPE_VMALLOC,
						NULL, 1024 * 1024, 1024 * 1024);
	if (result)
		return result;
	for (i = 0; i < NS7_AUDIO_URBS; i++) {
		chip->playback[i].chip = chip;
		chip->playback[i].urb =
		    ns7_iso_urb(chip, 2, 8, 156, 1, ns7_playback_complete,
				&chip->playback[i]);
		chip->capture[i].chip = chip;
		chip->capture[i].urb = usb_alloc_urb(0, GFP_KERNEL);
		if (!chip->playback[i].urb || !chip->capture[i].urb)
			return -ENOMEM;
		u8 *buffer = kmalloc(512, GFP_KERNEL);
		if (!buffer)
			return -ENOMEM;
		usb_fill_bulk_urb(chip->capture[i].urb, chip->usb,
				  usb_rcvbulkpipe(chip->usb, 6), buffer, 512,
				  ns7_capture_complete, &chip->capture[i]);
	}
	for (i = 0; i < NS7_FEEDBACK_URBS; i++) {
		chip->feedback[i] = ns7_iso_urb(chip, 0x81, 1, 3, 8,
						ns7_feedback_complete, chip);
		if (!chip->feedback[i])
			return -ENOMEM;
	}
	return 0;
}

static void ns7_read(struct urb *urb)
{
	struct ns7 *chip = urb->context;
	unsigned long flags;
	u8 bytes[NS7_PACKET];
	u8 *wire = urb->transfer_buffer;
	int count, result;

	if (urb->status == -ENOENT || urb->status == -ECONNRESET ||
	    urb->status == -ESHUTDOWN || READ_ONCE(chip->dead) ||
	    READ_ONCE(chip->configuring))
		return;
	if (!urb->status) {
		count = ns7_packet_read(wire, urb->actual_length, bytes);
		atomic64_add(count, &chip->received);
		spin_lock_irqsave(&chip->lock, flags);
		if (chip->input && count)
			snd_rawmidi_receive(chip->input, bytes, count);
		spin_unlock_irqrestore(&chip->lock, flags);
	} else {
		atomic64_inc(&chip->errors);
	}
	result = usb_submit_urb(urb, GFP_ATOMIC);
	if (result)
		atomic64_inc(&chip->errors);
}

static void ns7_write(struct urb *urb);

static void ns7_send_locked(struct ns7 *chip)
{
	int count, result;

	if (chip->dead || chip->configuring || chip->writing ||
	    !chip->output_enabled || !chip->output)
		return;
	ns7_packet_init(chip->write_buffer);
	count = snd_rawmidi_transmit_peek(chip->output, chip->write_buffer, NS7_PAYLOAD);
	if (count <= 0)
		return;
	chip->pending = count;
	chip->writing = true;
	result = usb_submit_urb(chip->write, GFP_ATOMIC);
	if (result) {
		chip->writing = false;
		chip->output_enabled = false;
		atomic64_inc(&chip->errors);
	}
}

static void ns7_write(struct urb *urb)
{
	struct ns7 *chip = urb->context;
	unsigned long flags;

	spin_lock_irqsave(&chip->lock, flags);
	chip->writing = false;
	if (!urb->status && urb->actual_length == NS7_PACKET) {
		if (chip->output)
			snd_rawmidi_transmit_ack(chip->output, chip->pending);
		atomic64_add(chip->pending, &chip->sent);
		ns7_send_locked(chip);
	} else if (!chip->dead && !chip->configuring && chip->output_enabled) {
		chip->output_enabled = false;
		atomic64_inc(&chip->errors);
	}
	spin_unlock_irqrestore(&chip->lock, flags);
}

static int ns7_open(struct snd_rawmidi_substream *stream)
{
	struct ns7 *chip = stream->rmidi->private_data;
	return READ_ONCE(chip->dead) ? -ENODEV : 0;
}

static int ns7_input_close(struct snd_rawmidi_substream *stream)
{
	struct ns7 *chip = stream->rmidi->private_data;
	unsigned long flags;
	spin_lock_irqsave(&chip->lock, flags);
	chip->input = NULL;
	spin_unlock_irqrestore(&chip->lock, flags);
	return 0;
}

static void ns7_input_trigger(struct snd_rawmidi_substream *stream, int up)
{
	struct ns7 *chip = stream->rmidi->private_data;
	unsigned long flags;
	spin_lock_irqsave(&chip->lock, flags);
	chip->input = up && !chip->dead ? stream : NULL;
	spin_unlock_irqrestore(&chip->lock, flags);
}

static int ns7_output_close(struct snd_rawmidi_substream *stream)
{
	struct ns7 *chip = stream->rmidi->private_data;
	unsigned long flags;
	spin_lock_irqsave(&chip->lock, flags);
	chip->output_enabled = false;
	spin_unlock_irqrestore(&chip->lock, flags);
	usb_kill_urb(chip->write);
	spin_lock_irqsave(&chip->lock, flags);
	chip->output = NULL;
	spin_unlock_irqrestore(&chip->lock, flags);
	return 0;
}

static void ns7_output_trigger(struct snd_rawmidi_substream *stream, int up)
{
	struct ns7 *chip = stream->rmidi->private_data;
	unsigned long flags;
	spin_lock_irqsave(&chip->lock, flags);
	chip->output_enabled = up;
	if (up) {
		chip->output = stream;
		ns7_send_locked(chip);
	}
	spin_unlock_irqrestore(&chip->lock, flags);
}

static const struct snd_rawmidi_ops ns7_input_ops = {
    .open = ns7_open,
    .close = ns7_input_close,
    .trigger = ns7_input_trigger,
};

static const struct snd_rawmidi_ops ns7_output_ops = {
    .open = ns7_open,
    .close = ns7_output_close,
    .trigger = ns7_output_trigger,
};

static void ns7_stop(struct ns7 *chip)
{
	unsigned long flags;
	int i;
	spin_lock_irqsave(&chip->lock, flags);
	chip->dead = true;
	chip->output_enabled = false;
	spin_unlock_irqrestore(&chip->lock, flags);
	for (i = 0; i < NS7_READS; i++)
		usb_kill_urb(chip->reads[i]);
	usb_kill_urb(chip->write);
	ns7_audio_stop(chip);
}

static void ns7_free(struct snd_card *card)
{
	struct ns7 *chip = card->private_data;
	int i;
	ns7_stop(chip);
	if (chip->auxiliary) {
		usb_driver_release_interface(&ns7_driver, chip->auxiliary);
		usb_put_intf(chip->auxiliary);
	}
	if (chip->configured && chip->usb->state != USB_STATE_NOTATTACHED)
		usb_control_msg(chip->usb, usb_sndctrlpipe(chip->usb, 0), 0x49,
				0x40, (u16)(s16)(s8)chip->saved_status, 0,
				NULL, 0, 1000);
	for (i = 0; i < NS7_READS; i++) {
		if (chip->reads[i])
			kfree(chip->reads[i]->transfer_buffer);
		usb_free_urb(chip->reads[i]);
	}
	usb_free_urb(chip->write);
	kfree(chip->write_buffer);
	for (i = 0; i < NS7_AUDIO_URBS; i++) {
		if (chip->playback[i].urb)
			kfree(chip->playback[i].urb->transfer_buffer);
		if (chip->capture[i].urb)
			kfree(chip->capture[i].urb->transfer_buffer);
		usb_free_urb(chip->playback[i].urb);
		usb_free_urb(chip->capture[i].urb);
	}
	for (i = 0; i < NS7_FEEDBACK_URBS; i++) {
		if (chip->feedback[i])
			kfree(chip->feedback[i]->transfer_buffer);
		usb_free_urb(chip->feedback[i]);
	}
	usb_put_dev(chip->usb);
}

static bool ns7_endpoint(struct usb_host_interface *alternate, u8 address,
			 unsigned int type, unsigned int packet,
			 unsigned int interval)
{
	for (int i = 0; i < alternate->desc.bNumEndpoints; i++) {
		const struct usb_endpoint_descriptor *endpoint =
		    &alternate->endpoint[i].desc;
		if (endpoint->bEndpointAddress == address)
			return usb_endpoint_type(endpoint) == type &&
			       usb_endpoint_maxp(endpoint) >= packet &&
			       (!interval || endpoint->bInterval == interval);
	}
	return false;
}

static bool ns7_layout(struct usb_device *usb, struct usb_interface *interface)
{
	struct usb_interface *auxiliary = usb_ifnum_to_if(usb, 1);
	struct usb_host_interface *midi =
	    usb_altnum_to_altsetting(interface, 1);
	struct usb_host_interface *audio = auxiliary
	    ? usb_altnum_to_altsetting(auxiliary, 1) : NULL;
	return usb->speed == USB_SPEED_HIGH && midi && audio &&
	       midi->desc.bNumEndpoints == 3 &&
	       audio->desc.bNumEndpoints == 2 &&
	       ns7_endpoint(midi, 0x02, USB_ENDPOINT_XFER_ISOC, 156, 1) &&
	       ns7_endpoint(midi, 0x83, USB_ENDPOINT_XFER_BULK, 512, 0) &&
	       ns7_endpoint(midi, 0x04, USB_ENDPOINT_XFER_BULK, 512, 0) &&
	       ns7_endpoint(audio, 0x81, USB_ENDPOINT_XFER_ISOC, 3, 4) &&
	       ns7_endpoint(audio, 0x86, USB_ENDPOINT_XFER_BULK, 512, 0);
}

static int ns7_probe(struct usb_interface *interface,
		     const struct usb_device_id *id)
{
	struct usb_device *usb = interface_to_usbdev(interface);
	struct snd_card *card;
	struct snd_rawmidi *midi;
	struct ns7 *chip;
	u8 *bytes;
	int result, i;

	if (interface->cur_altsetting->desc.bInterfaceNumber != 0)
		return -ENODEV;
	if (!ns7_layout(usb, interface))
		return -ENODEV;
	result = snd_card_new(&interface->dev, -1, "NS7", THIS_MODULE,
			      sizeof(*chip), &card);
	if (result)
		return result;
	chip = card->private_data;
	chip->card = card;
	chip->usb = usb_get_dev(usb);
	spin_lock_init(&chip->lock);
	mutex_init(&chip->audio_mutex);
	card->private_free = ns7_free;
	strscpy(card->driver, "snd_ns7", sizeof(card->driver));
	strscpy(card->shortname, "Numark NS7", sizeof(card->shortname));
	snprintf(card->longname, sizeof(card->longname),
		 "Original Numark NS7 at %s", dev_name(&usb->dev));
	result = usb_set_interface(usb, 0, 1);
	if (result)
		goto fail;
	if (!usb_ifnum_to_if(usb, 1)) {
		result = -ENODEV;
		goto fail;
	}
	chip->auxiliary = usb_get_intf(usb_ifnum_to_if(usb, 1));
	result = usb_driver_claim_interface(&ns7_driver, chip->auxiliary, chip);
	if (result) {
		usb_put_intf(chip->auxiliary);
		chip->auxiliary = NULL;
		goto fail;
	}
	result = usb_set_interface(usb, 1, 1);
	if (result)
		goto fail;
	bytes = kmalloc(1, GFP_KERNEL);
	if (!bytes) {
		result = -ENOMEM;
		goto fail;
	}
	result = usb_control_msg(usb, usb_rcvctrlpipe(usb, 0), 0x49, 0xc0, 0, 0,
				 bytes, 1, 1000);
	if (result == 1) {
		chip->saved_status = bytes[0];
		result = usb_control_msg(usb, usb_sndctrlpipe(usb, 0), 0x49,
					 0x40, (u16)(s16)(s8)(chip->saved_status | 0x30), 0,
					 NULL, 0, 1000);
		if (!result)
			chip->configured = true;
	} else if (result >= 0) {
		result = -EIO;
	}
	kfree(bytes);
	if (result)
		goto fail;
	result = snd_rawmidi_new(card, "Numark NS7 MIDI", 0, 1, 1, &midi);
	if (result)
		goto fail;
	midi->private_data = chip;
	strscpy(midi->name, "Numark NS7 MIDI", sizeof(midi->name));
	midi->info_flags = SNDRV_RAWMIDI_INFO_OUTPUT |
			   SNDRV_RAWMIDI_INFO_INPUT | SNDRV_RAWMIDI_INFO_DUPLEX;
	snd_rawmidi_set_ops(midi, SNDRV_RAWMIDI_STREAM_INPUT, &ns7_input_ops);
	snd_rawmidi_set_ops(midi, SNDRV_RAWMIDI_STREAM_OUTPUT, &ns7_output_ops);
	chip->write = usb_alloc_urb(0, GFP_KERNEL);
	chip->write_buffer = kmalloc(NS7_PACKET, GFP_KERNEL);
	if (!chip->write || !chip->write_buffer) {
		result = -ENOMEM;
		goto fail;
	}
	usb_fill_bulk_urb(chip->write, usb, usb_sndbulkpipe(usb, 4),
			  chip->write_buffer, NS7_PACKET, ns7_write, chip);
	for (i = 0; i < NS7_READS; i++) {
		chip->reads[i] = usb_alloc_urb(0, GFP_KERNEL);
		bytes = kmalloc(NS7_PACKET, GFP_KERNEL);
		if (!chip->reads[i] || !bytes) {
			kfree(bytes);
			result = -ENOMEM;
			goto fail;
		}
		usb_fill_bulk_urb(chip->reads[i], usb, usb_rcvbulkpipe(usb, 3),
				  bytes, NS7_PACKET, ns7_read, chip);
		result = usb_submit_urb(chip->reads[i], GFP_KERNEL);
		if (result)
			goto fail;
	}
	result = ns7_pcm_new(chip);
	if (result)
		goto fail;
	result = snd_card_register(card);
	if (result)
		goto fail;
	usb_set_intfdata(interface, chip);
	dev_info(&interface->dev,
		 "original NS7 duplex MIDI and PCM registered\n");
	return 0;
fail:
	snd_card_free(card);
	usb_set_interface(usb, 0, 0);
	return result;
}

static void ns7_disconnect(struct usb_interface *interface)
{
	struct ns7 *chip = usb_get_intfdata(interface);
	usb_set_intfdata(interface, NULL);
	if (interface->cur_altsetting->desc.bInterfaceNumber != 0)
		return;
	if (!chip)
		return;
	snd_card_disconnect(chip->card);
	ns7_stop(chip);
	dev_info(&interface->dev,
		 "NS7 MIDI bytes: input=%lld output=%lld errors=%lld\n",
		 atomic64_read(&chip->received), atomic64_read(&chip->sent),
		 atomic64_read(&chip->errors));
	snd_card_free_when_closed(chip->card);
}

static int ns7_pre_reset(struct usb_interface *interface)
{
	struct ns7 *chip = usb_get_intfdata(interface);
	struct snd_pcm_substream *streams[2];
	unsigned long flags;
	if (!chip || interface->cur_altsetting->desc.bInterfaceNumber != 0)
		return 0;
	mutex_lock(&chip->audio_mutex);
	spin_lock_irqsave(&chip->lock, flags);
	for (int i = 0; i < 2; i++) {
		streams[i] = chip->pcm[i].stream;
		chip->pcm[i].running = false;
		chip->pcm[i].prepared = false;
		chip->pcm[i].generation++;
	}
	spin_unlock_irqrestore(&chip->lock, flags);
	ns7_audio_stop(chip);
	ns7_midi_pause(chip);
	for (int i = 0; i < 2; i++)
		if (streams[i])
			snd_pcm_stop_xrun(streams[i]);
	return 0;
}

static int ns7_post_reset(struct usb_interface *interface)
{
	struct ns7 *chip = usb_get_intfdata(interface);
	u8 *status;
	int result;
	if (!chip || interface->cur_altsetting->desc.bInterfaceNumber != 0)
		return 0;
	status = kmalloc(1, GFP_KERNEL);
	if (!status) {
		result = -ENOMEM;
		goto unlock;
	}
	result = usb_control_msg(chip->usb, usb_rcvctrlpipe(chip->usb, 0), 0x49,
				 0xc0, 0, 0, status, 1, 1000);
	if (result == 1)
		result = usb_control_msg(chip->usb, usb_sndctrlpipe(chip->usb, 0),
					 0x49, 0x40, (u16)(s16)(s8)(status[0] | 0x30),
					 0, NULL, 0, 1000);
	else if (result >= 0)
		result = -EIO;
	kfree(status);
	if (!result)
		result = ns7_midi_resume(chip);
unlock:
	mutex_unlock(&chip->audio_mutex);
	return result;
}

static ssize_t midi_input_bytes_show(struct device *device,
				     struct device_attribute *attr,
				     char *buffer)
{
	struct ns7 *chip = dev_get_drvdata(device);
	return sysfs_emit(buffer, "%lld\n", atomic64_read(&chip->received));
}
static DEVICE_ATTR_RO(midi_input_bytes);

static ssize_t midi_output_bytes_show(struct device *device,
				      struct device_attribute *attr,
				      char *buffer)
{
	struct ns7 *chip = dev_get_drvdata(device);
	return sysfs_emit(buffer, "%lld\n", atomic64_read(&chip->sent));
}
static DEVICE_ATTR_RO(midi_output_bytes);

static ssize_t midi_errors_show(struct device *device,
				struct device_attribute *attr, char *buffer)
{
	struct ns7 *chip = dev_get_drvdata(device);
	return sysfs_emit(buffer, "%lld\n", atomic64_read(&chip->errors));
}
static DEVICE_ATTR_RO(midi_errors);

static ssize_t pcm_playback_frames_show(struct device *device,
					struct device_attribute *attr,
					char *buffer)
{
	struct ns7 *chip = dev_get_drvdata(device);
	return sysfs_emit(buffer, "%lld\n",
			  atomic64_read(&chip->playback_frames));
}
static DEVICE_ATTR_RO(pcm_playback_frames);

static ssize_t pcm_capture_frames_show(struct device *device,
				       struct device_attribute *attr,
				       char *buffer)
{
	struct ns7 *chip = dev_get_drvdata(device);
	return sysfs_emit(buffer, "%lld\n",
			  atomic64_read(&chip->capture_frames));
}
static DEVICE_ATTR_RO(pcm_capture_frames);

static ssize_t pcm_feedback_frames_show(struct device *device,
					struct device_attribute *attr,
					char *buffer)
{
	struct ns7 *chip = dev_get_drvdata(device);
	return sysfs_emit(buffer, "%lld\n",
			  atomic64_read(&chip->feedback_frames));
}
static DEVICE_ATTR_RO(pcm_feedback_frames);

static ssize_t pcm_feedback_idle_show(struct device *device,
				      struct device_attribute *attr, char *buffer)
{
	struct ns7 *chip = dev_get_drvdata(device);
	return sysfs_emit(buffer, "%lld\n", atomic64_read(&chip->feedback_idle));
}
static DEVICE_ATTR_RO(pcm_feedback_idle);

static ssize_t pcm_errors_show(struct device *device,
			       struct device_attribute *attr, char *buffer)
{
	struct ns7 *chip = dev_get_drvdata(device);
	return sysfs_emit(buffer, "%lld\n", atomic64_read(&chip->audio_errors));
}
static DEVICE_ATTR_RO(pcm_errors);

static struct attribute *ns7_attrs[] = {
    &dev_attr_midi_input_bytes.attr,   &dev_attr_midi_output_bytes.attr,
    &dev_attr_midi_errors.attr,	       &dev_attr_pcm_playback_frames.attr,
    &dev_attr_pcm_capture_frames.attr, &dev_attr_pcm_feedback_frames.attr,
    &dev_attr_pcm_errors.attr, &dev_attr_pcm_feedback_idle.attr, NULL,
};
ATTRIBUTE_GROUPS(ns7);

static const struct usb_device_id ns7_ids[] = {
    {USB_DEVICE_INTERFACE_NUMBER(0x15e4, 0x0071, 0)}, {}};
MODULE_DEVICE_TABLE(usb, ns7_ids);

static struct usb_driver ns7_driver = {
    .name = "snd_ns7",
    .probe = ns7_probe,
    .disconnect = ns7_disconnect,
    .pre_reset = ns7_pre_reset,
    .post_reset = ns7_post_reset,
    .id_table = ns7_ids,
    .dev_groups = ns7_groups,
};
module_usb_driver(ns7_driver);
MODULE_LICENSE("GPL");
MODULE_VERSION("0.1.0");
MODULE_AUTHOR("Omatainer contributors");
MODULE_DESCRIPTION("Original Numark NS7 vendor USB MIDI and PCM transport");
