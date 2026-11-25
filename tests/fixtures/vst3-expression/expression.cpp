#include "public.sdk/source/vst/vstaudioeffect.h"
#include "public.sdk/source/vst/vsteditcontroller.h"
#include "public.sdk/source/main/pluginfactory.h"
#include "pluginterfaces/vst/ivstevents.h"
#include "pluginterfaces/vst/ivstnoteexpression.h"
#include <algorithm>
#include <array>
#include <cmath>

using namespace Steinberg;
using namespace Steinberg::Vst;
namespace {
const FUID instrument_id(0x4F4D4154,0x41494E45,0x524D5045,0x00000001);
const FUID controller_id(0x4F4D4154,0x41494E45,0x524D5045,0x00000002);
struct Voice {
    int32 id = 0;
    bool held = false;
    double tuning = 0.5;
    double pressure = 0;
    double timbre = 64.0/127.0;
};
class Expression final : public AudioEffect {
    std::array<Voice,256> voices{};
public:
    Expression() { setControllerClass(controller_id); }
    static FUnknown* create(void*) { return static_cast<IAudioProcessor*>(new Expression); }
    tresult PLUGIN_API initialize(FUnknown* owner) override {
        if (AudioEffect::initialize(owner)!=kResultOk) return kResultFalse;
        addAudioOutput(STR16("Expression values"),SpeakerArr::k40Music);
        addEventInput(STR16("Independent notes"),16);
        return kResultOk;
    }
    tresult PLUGIN_API process(ProcessData& data) override {
        if (data.symbolicSampleSize!=kSample32 || data.numOutputs!=1 || data.outputs[0].numChannels!=4) return kResultFalse;
        for (int32 sample=0;sample<data.numSamples;++sample) {
            if (data.inputEvents) for (int32 index=0;index<data.inputEvents->getEventCount();++index) {
                Event event{};
                if (data.inputEvents->getEvent(index,event)!=kResultOk || event.sampleOffset!=sample) continue;
                if (event.type==Event::kNoteOnEvent) {
                    auto slot=std::find_if(voices.begin(),voices.end(),[](auto& voice){return !voice.held;});
                    if (slot==voices.end() || event.noteOn.noteId<=0) return kResultFalse;
                    *slot=Voice{};slot->id=event.noteOn.noteId;slot->held=true;
                }
                if (event.type==Event::kNoteOffEvent) for (auto& voice:voices) if (voice.id==event.noteOff.noteId) voice.held=false;
                if (event.type==Event::kNoteExpressionValueEvent) for (auto& voice:voices) if (voice.held && voice.id==event.noteExpressionValue.noteId) {
                    auto value=event.noteExpressionValue.value;
                    if (event.noteExpressionValue.typeId==kTuningTypeID) voice.tuning=value;
                    if (event.noteExpressionValue.typeId==kExpressionTypeID) voice.pressure=value;
                    if (event.noteExpressionValue.typeId==kBrightnessTypeID) voice.timbre=value;
                }
            }
            std::array<float,4> result{};
            for (const auto& voice:voices) if (voice.held) {
                result[0]+=0.01f;result[1]+=static_cast<float>(voice.tuning*0.1);
                result[2]+=static_cast<float>(voice.pressure*0.1);result[3]+=static_cast<float>(voice.timbre*0.1);
            }
            for (int channel=0;channel<4;++channel) data.outputs[0].channelBuffers32[channel][sample]=result[channel];
        }
        data.outputs[0].silenceFlags=0;return kResultOk;
    }
};
class Controls final : public EditController, public INoteExpressionController {
public:
    static FUnknown* create(void*) { return static_cast<IEditController*>(new Controls); }
    int32 PLUGIN_API getNoteExpressionCount(int32 bus,int16 channel) override { return bus==0 && channel==0 ? 3 : 0; }
    tresult PLUGIN_API getNoteExpressionInfo(int32 bus,int16 channel,int32 index,NoteExpressionTypeInfo& info) override {
        if (bus!=0 || channel!=0 || index<0 || index>=3) return kResultFalse;
        info={};info.typeId=std::array<NoteExpressionTypeID,3>{kTuningTypeID,kExpressionTypeID,kBrightnessTypeID}[index];
        const char* title=std::array<const char*,3>{"Pitch","Pressure","Timbre"}[index];
        for (int offset=0;title[offset];++offset) info.title[offset]=info.shortTitle[offset]=static_cast<TChar>(title[offset]);
        info.unitId=-1;info.valueDesc={index==0?0.5:0.0,0.0,1.0,0};
        info.flags=index==0?NoteExpressionTypeInfo::kIsBipolar:0;return kResultOk;
    }
    tresult PLUGIN_API getNoteExpressionStringByValue(int32,int16,NoteExpressionTypeID,NoteExpressionValue,String128) override { return kNotImplemented; }
    tresult PLUGIN_API getNoteExpressionValueByString(int32,int16,NoteExpressionTypeID,const TChar*,NoteExpressionValue&) override { return kNotImplemented; }
    OBJ_METHODS(Controls,EditController)
    DEFINE_INTERFACES
        DEF_INTERFACE(INoteExpressionController)
    END_DEFINE_INTERFACES(EditController)
    REFCOUNT_METHODS(EditController)
};
}
BEGIN_FACTORY_DEF("Omatainer","https://github.com/michaelmonetized/omatainer","")
DEF_CLASS2(INLINE_UID_FROM_FUID(instrument_id),PClassInfo::kManyInstances,kVstAudioEffectClass,"Omatainer Expression Contract",Vst::kDistributable,"Instrument|Synth","1.0.0",kVstVersionString,Expression::create)
DEF_CLASS2(INLINE_UID_FROM_FUID(controller_id),PClassInfo::kManyInstances,kVstComponentControllerClass,"Omatainer Expression Controls",0,"","1.0.0",kVstVersionString,Controls::create)
END_FACTORY
