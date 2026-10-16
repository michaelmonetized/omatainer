/* SPDX-License-Identifier: MIT */
#include <clap/clap.h>
#include <stdlib.h>
#include <string.h>

#ifndef BED_CHANNELS
#define BED_CHANNELS 6
#endif

typedef struct {
    clap_plugin_t plugin;
    float history[BED_CHANNELS + 1][19];
    float previous[BED_CHANNELS];
    unsigned cursor;
} Bed;

static bool init(const clap_plugin_t *p) { (void)p; return true; }
static void destroy(const clap_plugin_t *p) { free(p->plugin_data); }
static bool activate(const clap_plugin_t *p, double rate, uint32_t min, uint32_t max) {
    (void)p; return rate > 0 && min > 0 && max >= min;
}
static void deactivate(const clap_plugin_t *p) { (void)p; }
static bool start(const clap_plugin_t *p) { (void)p; return true; }
static void stop(const clap_plugin_t *p) { (void)p; }
static void reset(const clap_plugin_t *p) {
    Bed *b = p->plugin_data;
    memset(b->history, 0, sizeof(b->history));
    memset(b->previous, 0, sizeof(b->previous));
    b->cursor = 0;
}
static clap_process_status process(const clap_plugin_t *p, const clap_process_t *block) {
    Bed *b = p->plugin_data;
    if (block->audio_inputs_count != 2 || block->audio_outputs_count != 2 ||
        block->audio_inputs[0].channel_count != BED_CHANNELS ||
        block->audio_outputs[0].channel_count != BED_CHANNELS ||
        block->audio_inputs[1].channel_count != 1 || block->audio_outputs[1].channel_count != 1) abort();
    for (uint32_t frame = 0; frame < block->frames_count; ++frame) {
        const float side = block->audio_inputs[1].data32[0][frame];
        for (unsigned channel = 0; channel < BED_CHANNELS; ++channel) {
            const float input = block->audio_inputs[0].data32[channel][frame];
            block->audio_outputs[0].data32[channel][frame] = b->history[channel][b->cursor];
            b->history[channel][b->cursor] = 0.5f * input + 0.25f * b->previous[channel] + 0.125f * side;
            b->previous[channel] = input;
        }
        block->audio_outputs[1].data32[0][frame] = b->history[BED_CHANNELS][b->cursor];
        b->history[BED_CHANNELS][b->cursor] = -side;
        b->cursor = (b->cursor + 1) % 19;
    }
    return CLAP_PROCESS_CONTINUE;
}
static uint32_t audio_count(const clap_plugin_t *p, bool input) { (void)p; (void)input; return 2; }
static bool audio_get(const clap_plugin_t *p, uint32_t index, bool input, clap_audio_port_info_t *port) {
    (void)p;
    if (index > 1) return false;
    memset(port, 0, sizeof(*port));
    port->id = index;
    strcpy(port->name, index ? (input ? "Independent mono sidechain" : "Independent mono auxiliary") : "Declared primary bed");
    port->flags = index ? 0 : CLAP_AUDIO_PORT_IS_MAIN;
    port->channel_count = index ? 1 : BED_CHANNELS;
    port->port_type = index ? CLAP_PORT_MONO : NULL;
    port->in_place_pair = CLAP_INVALID_ID;
    return true;
}
static uint32_t latency(const clap_plugin_t *p) { (void)p; return 19; }
static uint32_t tail(const clap_plugin_t *p) { (void)p; return 1; }
static const clap_plugin_audio_ports_t audio = {audio_count, audio_get};
static const clap_plugin_latency_t delay = {latency};
static const clap_plugin_tail_t decay = {tail};
static const void *extension(const clap_plugin_t *p, const char *id) {
    (void)p;
    if (!strcmp(id, CLAP_EXT_AUDIO_PORTS)) return &audio;
    if (!strcmp(id, CLAP_EXT_LATENCY)) return &delay;
    if (!strcmp(id, CLAP_EXT_TAIL)) return &decay;
    return NULL;
}
static const char *features[] = {"audio-effect", NULL};
static const clap_plugin_descriptor_t descriptor = {CLAP_VERSION_INIT, "org.omatainer.conformance.speaker-bed", "Speaker bed equation", "Omatainer", "", "", "", "1", "Original primary bed and independent bus fixture", features};
static uint32_t plugin_count(const clap_plugin_factory_t *f) { (void)f; return 1; }
static const clap_plugin_descriptor_t *plugin_descriptor(const clap_plugin_factory_t *f, uint32_t i) { (void)f; return i ? NULL : &descriptor; }
static const clap_plugin_t *create(const clap_plugin_factory_t *f, const clap_host_t *host, const char *id) {
    (void)f; (void)host;
    if (strcmp(id, descriptor.id)) return NULL;
    Bed *b = calloc(1, sizeof(*b));
    if (!b) return NULL;
    b->plugin = (clap_plugin_t){&descriptor, b, init, destroy, activate, deactivate, start, stop, reset, process, extension, NULL};
    return &b->plugin;
}
static const clap_plugin_factory_t factory = {plugin_count, plugin_descriptor, create};
static bool entry_init(const char *path) { (void)path; return true; }
static void entry_deinit(void) {}
static const void *get_factory(const char *id) { return strcmp(id, CLAP_PLUGIN_FACTORY_ID) ? NULL : &factory; }
CLAP_EXPORT const clap_plugin_entry_t clap_entry = {CLAP_VERSION_INIT, entry_init, entry_deinit, get_factory};
