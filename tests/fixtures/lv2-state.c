/* SPDX-License-Identifier: MIT */
#include <lv2/core/lv2.h>
#include <lv2/atom/atom.h>
#include <lv2/atom/util.h>
#include <lv2/midi/midi.h>
#include <lv2/state/state.h>
#include <lv2/urid/urid.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    float *out;
    const LV2_Atom_Sequence *input;
    const float *gain;
    float *latency;
    LV2_Atom_Sequence *output;
    LV2_URID_Map *map;
    LV2_State_Map_Path *paths;
    LV2_State_Free_Path *free_path;
    char *path;
    float sample;
    bool held;
} Voice;
static LV2_Handle instantiate(const LV2_Descriptor *d, double rate, const char *bundle, const LV2_Feature *const *features) {
    (void)d; (void)rate; (void)bundle;
    Voice *v = calloc(1, sizeof(*v));
    LV2_State_Make_Path *make = NULL;
    for (unsigned i = 0; features && features[i]; ++i) {
        const LV2_Feature *f = features[i];
        if (!strcmp(f->URI, LV2_URID__map)) v->map = f->data;
        if (!strcmp(f->URI, LV2_STATE__mapPath)) v->paths = f->data;
        if (!strcmp(f->URI, LV2_STATE__makePath)) make = f->data;
        if (!strcmp(f->URI, LV2_STATE__freePath)) v->free_path = f->data;
    }
    if (!v->map || !v->paths || !v->free_path || !make) { free(v); return NULL; }
    v->path = make->path(make->handle, "nested/sample.bin");
    if (!v->path) { free(v); return NULL; }
    FILE *file = fopen(v->path, "wb");
    if (!file) { v->free_path->free_path(v->free_path->handle, v->path); free(v); return NULL; }
    const unsigned char sample = 32;
    fwrite(&sample, 1, 1, file);
    fclose(file);
    v->sample = 0.25f;
    return v;
}
static void connect(LV2_Handle handle, uint32_t port, void *data) {
    Voice *v = handle;
    switch (port) {
        case 0: v->out = data; break;
        case 1: v->input = data; break;
        case 2: v->gain = data; break;
        case 3: v->latency = data; break;
        case 4: v->output = data; break;
    }
}
static void run(LV2_Handle handle, uint32_t frames) {
    Voice *v = handle;
    const uint32_t midi = v->map->map(v->map->handle, LV2_MIDI__MidiEvent);
    const uint32_t capacity = v->output->atom.size;
    v->output->atom.type = v->map->map(v->map->handle, LV2_ATOM__Sequence);
    lv2_atom_sequence_clear(v->output);
    const LV2_Atom_Event *event = lv2_atom_sequence_begin(&v->input->body);
    for (uint32_t sample = 0; sample < frames; ++sample) {
        while (!lv2_atom_sequence_is_end(&v->input->body, v->input->atom.size, event) && event->time.frames <= sample) {
            if (event->body.type == midi && event->body.size == 3) {
                const uint8_t *data = LV2_ATOM_BODY_CONST(&event->body);
                if ((data[0] & 0xf0) == 0x90 && data[2]) v->held = true;
                if ((data[0] & 0xf0) == 0x80 || ((data[0] & 0xf0) == 0x90 && !data[2])) v->held = false;
                lv2_atom_sequence_append_event(v->output, capacity, event);
            }
            event = lv2_atom_sequence_next(event);
        }
        v->out[sample] = v->held ? *v->gain + v->sample : 0.0f;
    }
    *v->latency = 11;
}
static LV2_State_Status save(LV2_Handle handle, LV2_State_Store_Function store, LV2_State_Handle data, uint32_t flags, const LV2_Feature *const *features) {
    (void)flags; (void)features;
    Voice *v = handle;
    char *path = v->paths->abstract_path(v->paths->handle, v->path);
    if (!path) return LV2_STATE_ERR_NO_SPACE;
    uint32_t key = v->map->map(v->map->handle, "urn:omatainer:conformance:sample");
    uint32_t type = v->map->map(v->map->handle, LV2_ATOM__Path);
    LV2_State_Status result = store(data, key, path, strlen(path) + 1, type, LV2_STATE_IS_POD);
    v->free_path->free_path(v->free_path->handle, path);
    return result;
}
static LV2_State_Status restore(LV2_Handle handle, LV2_State_Retrieve_Function retrieve, LV2_State_Handle data, uint32_t flags, const LV2_Feature *const *features) {
    (void)flags; (void)features;
    Voice *v = handle;
    size_t size; uint32_t type, property_flags;
    const char *path = retrieve(data, v->map->map(v->map->handle, "urn:omatainer:conformance:sample"), &size, &type, &property_flags);
    if (!path) return LV2_STATE_SUCCESS;
    if (type != v->map->map(v->map->handle, LV2_ATOM__Path) || !size || path[size - 1]) return LV2_STATE_ERR_BAD_TYPE;
    char *absolute = v->paths->absolute_path(v->paths->handle, path);
    if (!absolute) return LV2_STATE_ERR_UNKNOWN;
    FILE *file = fopen(absolute, "rb");
    if (!file) { v->free_path->free_path(v->free_path->handle, absolute); return LV2_STATE_ERR_UNKNOWN; }
    unsigned char sample;
    size_t count = fread(&sample, 1, 1, file);
    fclose(file);
    if (count != 1) { v->free_path->free_path(v->free_path->handle, absolute); return LV2_STATE_ERR_UNKNOWN; }
    v->free_path->free_path(v->free_path->handle, v->path);
    v->path = absolute;
    v->sample = sample / 128.0f;
    return LV2_STATE_SUCCESS;
}
static void cleanup(LV2_Handle handle) { Voice *v = handle; v->free_path->free_path(v->free_path->handle, v->path); free(v); }
static const LV2_State_Interface state = {save, restore};
static const void *extension(const char *uri) { return strcmp(uri, LV2_STATE__interface) ? NULL : &state; }
static const LV2_Descriptor descriptor = {"urn:omatainer:conformance:lv2", instantiate, connect, NULL, run, NULL, cleanup, extension};
LV2_SYMBOL_EXPORT const LV2_Descriptor *lv2_descriptor(uint32_t index) { return index ? NULL : &descriptor; }
