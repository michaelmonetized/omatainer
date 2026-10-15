/* SPDX-License-Identifier: MIT */
#include <clap/clap.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    clap_plugin_t plugin;
    const clap_host_t *host;
    bool active;
    bool sounding;
    int32_t note;
    double gain;
    double pressure;
    double modulation;
    int64_t steady;
} Voice;

static Voice *voice(const clap_plugin_t *p) { return p->plugin_data; }
static void require(bool value) { if (!value) abort(); }
static const clap_host_thread_check_t *threads(Voice *v) { return v->host->get_extension(v->host, CLAP_EXT_THREAD_CHECK); }
static bool init(const clap_plugin_t *p) {
    Voice *v = voice(p);
    require(threads(v)->is_main_thread(v->host));
    v->host->get_extension(v->host, CLAP_EXT_THREAD_POOL);
    return true;
}
static void destroy(const clap_plugin_t *p) { free(voice(p)); }
static bool activate(const clap_plugin_t *p, double rate, uint32_t min, uint32_t max) {
    Voice *v = voice(p);
    require(threads(v)->is_main_thread(v->host) && rate > 0 && min > 0 && max >= min);
    v->active = true;
    return true;
}
static void deactivate(const clap_plugin_t *p) { voice(p)->active = false; }
static bool start(const clap_plugin_t *p) { require(threads(voice(p))->is_audio_thread(voice(p)->host)); return true; }
static void stop(const clap_plugin_t *p) { require(threads(voice(p))->is_audio_thread(voice(p)->host)); }
static void reset(const clap_plugin_t *p) { voice(p)->sounding = false; }
static clap_process_status process(const clap_plugin_t *p, const clap_process_t *block) {
    Voice *v = voice(p);
    require(v->active && threads(v)->is_audio_thread(v->host));
    require(block->steady_time >= 0 && (v->steady < 0 || block->steady_time > v->steady));
    v->steady = block->steady_time;
    uint32_t event = 0, count = block->in_events->size(block->in_events);
    for (uint32_t sample = 0; sample < block->frames_count; ++sample) {
        while (event < count) {
            const clap_event_header_t *h = block->in_events->get(block->in_events, event);
            if (h->time > sample) break;
            require(h->time == sample);
            if (h->type == CLAP_EVENT_NOTE_ON) {
                const clap_event_note_t *n = (const clap_event_note_t *)h;
                v->note = n->note_id;
                v->sounding = true;
            } else if (h->type == CLAP_EVENT_NOTE_OFF) {
                const clap_event_note_t *n = (const clap_event_note_t *)h;
                if (n->note_id == v->note) v->sounding = false;
            } else if (h->type == CLAP_EVENT_NOTE_EXPRESSION) {
                const clap_event_note_expression_t *n = (const clap_event_note_expression_t *)h;
                if (n->note_id == v->note && n->expression_id == CLAP_NOTE_EXPRESSION_PRESSURE) v->pressure = n->value;
            } else if (h->type == CLAP_EVENT_PARAM_MOD) {
                v->modulation = ((const clap_event_param_mod_t *)h)->amount;
            } else if (h->type == CLAP_EVENT_PARAM_VALUE) {
                v->gain = ((const clap_event_param_value_t *)h)->value;
            }
            ++event;
        }
        block->audio_outputs[0].data32[0][sample] = v->sounding ? (float)(v->gain + v->pressure + v->modulation) : 0.0f;
    }
    return CLAP_PROCESS_CONTINUE;
}
static uint32_t audio_count(const clap_plugin_t *p, bool input) { (void)p; return input ? 0 : 1; }
static bool audio_get(const clap_plugin_t *p, uint32_t i, bool input, clap_audio_port_info_t *port) {
    (void)p;
    if (input || i) return false;
    memset(port, 0, sizeof(*port));
    port->id = 0;
    strcpy(port->name, "Native output");
    port->flags = CLAP_AUDIO_PORT_IS_MAIN;
    port->channel_count = 1;
    port->port_type = CLAP_PORT_MONO;
    port->in_place_pair = CLAP_INVALID_ID;
    return true;
}
static uint32_t note_count(const clap_plugin_t *p, bool input) { (void)p; return input ? 1 : 0; }
static bool note_get(const clap_plugin_t *p, uint32_t i, bool input, clap_note_port_info_t *port) {
    (void)p;
    if (!input || i) return false;
    memset(port, 0, sizeof(*port));
    port->id = 0;
    strcpy(port->name, "Addressed notes");
    port->supported_dialects = port->preferred_dialect = CLAP_NOTE_DIALECT_CLAP;
    return true;
}
static uint32_t param_count(const clap_plugin_t *p) { (void)p; return 1; }
static bool param_info(const clap_plugin_t *p, uint32_t i, clap_param_info_t *info) {
    (void)p;
    if (i) return false;
    memset(info, 0, sizeof(*info));
    info->id = 7;
    strcpy(info->name, "Level");
    info->min_value = 0;
    info->max_value = 1;
    info->default_value = 0.25;
    info->flags = CLAP_PARAM_IS_AUTOMATABLE | CLAP_PARAM_IS_MODULATABLE | CLAP_PARAM_IS_MODULATABLE_PER_NOTE_ID | CLAP_PARAM_IS_MODULATABLE_PER_PORT | CLAP_PARAM_IS_MODULATABLE_PER_CHANNEL | CLAP_PARAM_IS_MODULATABLE_PER_KEY;
    return true;
}
static bool param_value(const clap_plugin_t *p, clap_id id, double *value) { if (id != 7) return false; *value = voice(p)->gain; return true; }
static bool state_save(const clap_plugin_t *p, const clap_ostream_t *stream) { return stream->write(stream, &voice(p)->gain, sizeof(double)) == sizeof(double); }
static bool state_load(const clap_plugin_t *p, const clap_istream_t *stream) {
    Voice *v = voice(p);
    if (stream->read(stream, &v->gain, sizeof(double)) != sizeof(double)) return false;
    const clap_host_params_t *params = v->host->get_extension(v->host, CLAP_EXT_PARAMS);
    params->rescan(v->host, CLAP_PARAM_RESCAN_VALUES | CLAP_PARAM_RESCAN_TEXT);
    return true;
}
static uint32_t latency(const clap_plugin_t *p) { require(voice(p)->active); return 19; }
static uint32_t tail(const clap_plugin_t *p) { (void)p; return 3; }
static const clap_plugin_audio_ports_t audio = {audio_count, audio_get};
static const clap_plugin_note_ports_t notes = {note_count, note_get};
static const clap_plugin_params_t params = {.count=param_count, .get_info=param_info, .get_value=param_value};
static const clap_plugin_state_t state = {state_save, state_load};
static const clap_plugin_latency_t delay = {latency};
static const clap_plugin_tail_t decay = {tail};
static const void *extension(const clap_plugin_t *p, const char *id) {
    (void)p;
    if (!strcmp(id, CLAP_EXT_AUDIO_PORTS)) return &audio;
    if (!strcmp(id, CLAP_EXT_NOTE_PORTS)) return &notes;
    if (!strcmp(id, CLAP_EXT_PARAMS)) return &params;
    if (!strcmp(id, CLAP_EXT_STATE)) return &state;
    if (!strcmp(id, CLAP_EXT_LATENCY)) return &delay;
    if (!strcmp(id, CLAP_EXT_TAIL)) return &decay;
    return NULL;
}
static const char *features[] = {"instrument", NULL};
static const clap_plugin_descriptor_t descriptor = {CLAP_VERSION_INIT, "org.omatainer.conformance.expression", "Native expression conformance", "Omatainer", "", "", "", "1", "Host ABI conformance fixture", features};
static uint32_t plugin_count(const clap_plugin_factory_t *f) { (void)f; return 1; }
static const clap_plugin_descriptor_t *plugin_descriptor(const clap_plugin_factory_t *f, uint32_t i) { (void)f; return i ? NULL : &descriptor; }
static const clap_plugin_t *create(const clap_plugin_factory_t *f, const clap_host_t *host, const char *id) {
    (void)f;
    if (strcmp(id, descriptor.id)) return NULL;
    Voice *v = calloc(1, sizeof(*v));
    if (!v) return NULL;
    v->host = host;
    v->gain = 0.25;
    v->steady = -1;
    v->plugin = (clap_plugin_t){&descriptor, v, init, destroy, activate, deactivate, start, stop, reset, process, extension, NULL};
    return &v->plugin;
}
static const clap_plugin_factory_t factory = {plugin_count, plugin_descriptor, create};
static bool entry_init(const char *path) { (void)path; return true; }
static void entry_deinit(void) {}
static const void *get_factory(const char *id) { return strcmp(id, CLAP_PLUGIN_FACTORY_ID) ? NULL : &factory; }
CLAP_EXPORT const clap_plugin_entry_t clap_entry = {CLAP_VERSION_INIT, entry_init, entry_deinit, get_factory};
