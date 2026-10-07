#include "public.sdk/source/vst/vstaudioeffect.h"
#include "public.sdk/source/vst/vsteditcontroller.h"
#include "public.sdk/source/main/pluginfactory.h"
#include "pluginterfaces/vst/ivstevents.h"
#include "pluginterfaces/vst/ivstparameterchanges.h"
#include "base/source/fstreamer.h"
#include <algorithm>
#include <array>
#include <cmath>

using namespace Steinberg;
using namespace Steinberg::Vst;
namespace {
const FUID effect_id(0x4F4D4154, 0x41494E45, 0x52465830, 0x00000001);
const FUID instrument_id(0x4F4D4154, 0x41494E45, 0x52494E53, 0x00000001);
const FUID controller_id(0x4F4D4154, 0x41494E45, 0x5243544C, 0x00000001);

class Contract final : public AudioEffect {
    bool instrument;
    double gain = 0.5;
    uint32 delay = 0;
    bool bypass = false;
    uint64 head = 0;
    std::array<std::array<float, 4096>, 2> history{};
    std::array<std::array<float, 128>, 16> voices{};
public:
    explicit Contract(bool musical) : instrument(musical) { setControllerClass(controller_id); }
    static FUnknown* effect(void*) { return static_cast<IAudioProcessor*>(new Contract(false)); }
    static FUnknown* synth(void*) { return static_cast<IAudioProcessor*>(new Contract(true)); }
    tresult PLUGIN_API initialize(FUnknown* owner) override {
        if (AudioEffect::initialize(owner) != kResultOk) return kResultFalse;
        if (!instrument) {
            addAudioInput(STR16("Main"), SpeakerArr::kStereo);
            addAudioInput(STR16("Sidechain"), SpeakerArr::kMono, kAux, 0);
        }
        addAudioOutput(STR16("Main"), SpeakerArr::kStereo);
        addAudioOutput(STR16("Context"), SpeakerArr::kMono, kAux, 0);
        addEventInput(STR16("Notes"), 16);
        return kResultOk;
    }
    tresult PLUGIN_API setBusArrangements(SpeakerArrangement* in, int32 ins, SpeakerArrangement* out, int32 outs) override {
        if (outs != 2 || out[0] != SpeakerArr::kStereo || out[1] != SpeakerArr::kMono) return kResultFalse;
        if (instrument ? ins != 0 : ins != 2 || in[0] != SpeakerArr::kStereo || in[1] != SpeakerArr::kMono) return kResultFalse;
        return AudioEffect::setBusArrangements(in,ins,out,outs);
    }
    uint32 PLUGIN_API getProcessContextRequirements() override {
        return IProcessContextRequirements::kNeedContinousTimeSamples | IProcessContextRequirements::kNeedProjectTimeMusic | IProcessContextRequirements::kNeedTempo | IProcessContextRequirements::kNeedTimeSignature | IProcessContextRequirements::kNeedTransportState;
    }
    uint32 PLUGIN_API getLatencySamples() override { return delay; }
    uint32 PLUGIN_API getTailSamples() override { return delay; }
    tresult PLUGIN_API setState(IBStream* state) override {
        IBStreamer stream(state,kLittleEndian);
        double next_gain, next_delay, next_bypass;
        if (!stream.readDouble(next_gain) || !stream.readDouble(next_delay) || !stream.readDouble(next_bypass)) return kResultFalse;
        if (!std::isfinite(next_gain) || next_gain < 0 || next_gain > 1 || (next_delay != 0 && next_delay != 64) || (next_bypass != 0 && next_bypass != 1)) return kResultFalse;
        gain = next_gain; delay = static_cast<uint32>(next_delay); bypass = next_bypass != 0;
        return kResultOk;
    }
    tresult PLUGIN_API getState(IBStream* state) override {
        IBStreamer stream(state,kLittleEndian);
        return stream.writeDouble(gain) && stream.writeDouble(delay) && stream.writeDouble(bypass ? 1. : 0.) ? kResultOk : kResultFalse;
    }
    tresult PLUGIN_API process(ProcessData& data) override {
        if (data.symbolicSampleSize != kSample32 || data.numOutputs != 2 || data.outputs[0].numChannels != 2 || data.outputs[1].numChannels != 1) return kResultFalse;
        if (!instrument && (data.numInputs != 2 || data.inputs[0].numChannels != 2 || data.inputs[1].numChannels != 1)) return kResultFalse;
        for (int32 frame = 0; frame < data.numSamples; ++frame) {
            if (auto* changes = data.inputParameterChanges) {
                for (int32 lane = 0; lane < changes->getParameterCount(); ++lane) {
                    auto* queue = changes->getParameterData(lane);
                    for (int32 point = 0; point < queue->getPointCount(); ++point) {
                        int32 offset; ParamValue value;
                        if (queue->getPoint(point,offset,value) == kResultOk && offset == frame) {
                            if (queue->getParameterId() == 0) gain = std::clamp(value,0.,1.);
                            if (queue->getParameterId() == 1) delay = value >= 0.5 ? 64 : 0;
                            if (queue->getParameterId() == 2) bypass = value >= 0.5;
                        }
                    }
                }
            }
            if (auto* events = data.inputEvents) {
                for (int32 index = 0; index < events->getEventCount(); ++index) {
                    Event event{};
                    if (events->getEvent(index,event) != kResultOk || event.sampleOffset != frame) continue;
                    if (event.type == Event::kNoteOnEvent) voices[event.noteOn.channel & 15][event.noteOn.pitch & 127] = event.noteOn.velocity;
                    if (event.type == Event::kNoteOffEvent) voices[event.noteOff.channel & 15][event.noteOff.pitch & 127] = 0;
                }
            }
            float note = 0;
            for (const auto& channel : voices) for (float velocity : channel) note += velocity * 0.1f;
            for (int32 channel = 0; channel < 2; ++channel) {
                float value = instrument ? note : data.inputs[0].channelBuffers32[channel][frame] + 2.f * data.inputs[1].channelBuffers32[0][frame];
                history[channel][head % 4096] = value;
                data.outputs[0].channelBuffers32[channel][frame] = head < delay ? 0.f : history[channel][(head + 4096 - delay) % 4096] * static_cast<float>(bypass ? 1. : gain);
            }
            const auto* context = data.processContext;
            data.outputs[1].channelBuffers32[0][frame] = context ? static_cast<float>((context->projectTimeSamples + frame) * 0.0000001 + context->projectTimeMusic * 0.001 + context->tempo * 0.00001 + context->timeSigNumerator * 0.0001 + context->timeSigDenominator * 0.00001 + ((context->state & ProcessContext::kPlaying) ? 0.01 : 0.)) : -1.f;
            ++head;
        }
        for (int32 bus = 0; bus < data.numOutputs; ++bus) data.outputs[bus].silenceFlags = 0;
        return kResultOk;
    }
};
class Controls final : public EditController {
public:
    static FUnknown* create(void*) { return static_cast<IEditController*>(new Controls); }
    tresult PLUGIN_API initialize(FUnknown* owner) override {
        if (EditController::initialize(owner) != kResultOk) return kResultFalse;
        parameters.addParameter(STR16("Gain"),nullptr,0,0.5,ParameterInfo::kCanAutomate,0);
        parameters.addParameter(STR16("Latency 64 samples"),nullptr,1,0.,ParameterInfo::kCanAutomate,1);
        parameters.addParameter(STR16("Bypass"),nullptr,1,0.,ParameterInfo::kCanAutomate | ParameterInfo::kIsBypass,2);
        return kResultOk;
    }
    tresult PLUGIN_API setComponentState(IBStream* state) override {
        IBStreamer stream(state,kLittleEndian); double gain,delay,bypass;
        if (!stream.readDouble(gain) || !stream.readDouble(delay) || !stream.readDouble(bypass)) return kResultFalse;
        setParamNormalized(0,gain); setParamNormalized(1,delay > 0 ? 1. : 0.); setParamNormalized(2,bypass);
        return kResultOk;
    }
};
}
BEGIN_FACTORY_DEF("Omatainer","https://github.com/michaelmonetized/omatainer","")
DEF_CLASS2(INLINE_UID_FROM_FUID(effect_id),PClassInfo::kManyInstances,kVstAudioEffectClass,"Omatainer Graph Contract",Vst::kDistributable,"Fx","1.0.0",kVstVersionString,Contract::effect)
DEF_CLASS2(INLINE_UID_FROM_FUID(instrument_id),PClassInfo::kManyInstances,kVstAudioEffectClass,"Omatainer Note Contract",Vst::kDistributable,"Instrument|Synth","1.0.0",kVstVersionString,Contract::synth)
DEF_CLASS2(INLINE_UID_FROM_FUID(controller_id),PClassInfo::kManyInstances,kVstComponentControllerClass,"Omatainer Contract Controls",0,"","1.0.0",kVstVersionString,Controls::create)
END_FACTORY
