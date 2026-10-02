#import <Foundation/Foundation.h>
#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <AppKit/AppKit.h>
#include <stdint.h>
#include <stdio.h>

static void error_text(char *out, size_t cap, NSString *text) {
    if (out && cap) snprintf(out, cap, "%s", text.UTF8String ?: "Unknown audio error");
}

typedef struct { ExtAudioFileRef file; AudioStreamBasicDescription format; int64_t frames; } Reader;
void *sonora_reader_open(const char *path, double *rate, uint32_t *channels, uint64_t *frames, char *err, size_t cap) {
    @autoreleasepool {
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        Reader *r = calloc(1, sizeof(Reader));
        OSStatus status = ExtAudioFileOpenURL((__bridge CFURLRef)url, &r->file);
        if (!status) {
            UInt32 n = sizeof(r->format);
            status = ExtAudioFileGetProperty(r->file, kExtAudioFileProperty_FileDataFormat, &n, &r->format);
        }
        if (!status) { UInt32 n = sizeof(r->frames); status = ExtAudioFileGetProperty(r->file, kExtAudioFileProperty_FileLengthFrames, &n, &r->frames); }
        if (!status && (r->format.mChannelsPerFrame == 0 || r->format.mChannelsPerFrame > 32 || r->format.mSampleRate <= 0 || r->frames <= 0)) status = -50;
        if (!status) {
            AudioStreamBasicDescription client = {0};
            client.mSampleRate = r->format.mSampleRate;
            client.mFormatID = kAudioFormatLinearPCM;
            client.mFormatFlags = kAudioFormatFlagsNativeFloatPacked;
            client.mBitsPerChannel = 32;
            client.mChannelsPerFrame = r->format.mChannelsPerFrame;
            client.mFramesPerPacket = 1;
            client.mBytesPerFrame = client.mBytesPerPacket = 4 * client.mChannelsPerFrame;
            status = ExtAudioFileSetProperty(r->file, kExtAudioFileProperty_ClientDataFormat, sizeof(client), &client);
        }
        if (status) {
            error_text(err, cap, [NSString stringWithFormat:@"Cannot decode this audio file (Core Audio %d). Try WAV, AIFF, MP3, M4A, CAF, or FLAC.", (int)status]);
            if (r->file) ExtAudioFileDispose(r->file);
            free(r); return NULL;
        }
        *rate = r->format.mSampleRate; *channels = r->format.mChannelsPerFrame; *frames = (uint64_t)r->frames;
        return r;
    }
}
int sonora_reader_seek(void *ptr, uint64_t frame) { return ExtAudioFileSeek(((Reader *)ptr)->file, (SInt64)frame); }
int sonora_reader_read(void *ptr, float *samples, uint32_t capacity, uint32_t *read) {
    Reader *r = ptr;
    AudioBufferList buffers = { .mNumberBuffers = 1, .mBuffers = {{ .mNumberChannels = r->format.mChannelsPerFrame, .mDataByteSize = capacity * r->format.mChannelsPerFrame * 4, .mData = samples }} };
    *read = capacity;
    return ExtAudioFileRead(r->file, read, &buffers);
}
void sonora_reader_close(void *ptr) { Reader *r = ptr; if (r) { ExtAudioFileDispose(r->file); free(r); } }

@interface SonoraPlayer : NSObject
@property AVAudioEngine *engine;
@property AVAudioPlayerNode *node;
@property AVAudioUnitEQ *gain;
@property AVAudioFile *file;
@property double start;
@property double end;
@property double paused;
@property BOOL running;
@end
@implementation SonoraPlayer
@end
void *sonora_player_new(void) {
    @autoreleasepool {
        SonoraPlayer *p = [SonoraPlayer new];
        p.engine = [AVAudioEngine new]; p.node = [AVAudioPlayerNode new]; p.gain = [[AVAudioUnitEQ alloc] initWithNumberOfBands:0];
        [p.engine attachNode:p.node]; [p.engine attachNode:p.gain];
        return (__bridge_retained void *)p;
    }
}
int sonora_player_load(void *ptr, const char *path, char *err, size_t cap) {
    @autoreleasepool {
        SonoraPlayer *p = (__bridge SonoraPlayer *)ptr;
        [p.node stop]; [p.engine stop]; p.running = NO; p.paused = 0;
        NSError *error = nil;
        p.file = [[AVAudioFile alloc] initForReading:[NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil] error:&error];
        if (!p.file) { error_text(err, cap, error.localizedDescription); return -1; }
        [p.engine disconnectNodeOutput:p.node]; [p.engine disconnectNodeOutput:p.gain];
        @try {
            [p.engine connect:p.node to:p.gain format:p.file.processingFormat];
            [p.engine connect:p.gain to:p.engine.mainMixerNode format:p.file.processingFormat];
        } @catch (NSException *e) { error_text(err, cap, e.reason); return -1; }
        return 0;
    }
}
int sonora_player_play(void *ptr, double start, double end, float db, char *err, size_t cap) {
    @autoreleasepool {
        SonoraPlayer *p = (__bridge SonoraPlayer *)ptr;
        if (!p.file) { error_text(err, cap, @"Open an audio file first."); return -1; }
        double rate = p.file.processingFormat.sampleRate;
        int64_t first = llround(start * rate), last = MIN(p.file.length, llround(end * rate));
        if (first < 0 || last <= first || last - first > UINT32_MAX) { error_text(err, cap, @"Choose a valid playback range shorter than 24 hours."); return -1; }
        [p.node stop]; p.running = NO;
        p.start = (double)first / rate; p.end = (double)last / rate; p.paused = p.start; p.gain.globalGain = db;
        NSError *error = nil;
        @try {
            [p.node scheduleSegment:p.file startingFrame:first frameCount:(AVAudioFrameCount)(last-first) atTime:nil completionHandler:nil];
            if (!p.engine.isRunning && ![p.engine startAndReturnError:&error]) { error_text(err, cap, error.localizedDescription); return -1; }
            [p.node play]; p.running = YES;
        } @catch (NSException *e) { error_text(err, cap, e.reason); return -1; }
        return 0;
    }
}
double sonora_player_position(void *ptr) {
    SonoraPlayer *p = (__bridge SonoraPlayer *)ptr;
    if (!p.running) return p.paused;
    AVAudioTime *render = p.node.lastRenderTime;
    AVAudioTime *time = render ? [p.node playerTimeForNodeTime:render] : nil;
    if (!time || !time.isSampleTimeValid || time.sampleRate <= 0) return p.start;
    return MIN(p.end, p.start + MAX(0, (double)time.sampleTime / time.sampleRate));
}
int sonora_player_is_playing(void *ptr) {
    SonoraPlayer *p = (__bridge SonoraPlayer *)ptr;
    if (!p.running) return 0;
    if (sonora_player_position(ptr) >= p.end) {
        p.paused = p.end; p.running = NO; [p.node stop]; [p.engine pause]; return 0;
    }
    return 1;
}
void sonora_player_pause(void *ptr) {
    SonoraPlayer *p = (__bridge SonoraPlayer *)ptr;
    p.paused = sonora_player_position(ptr); p.running = NO; [p.node pause]; [p.engine pause];
}
void sonora_player_stop(void *ptr) {
    SonoraPlayer *p = (__bridge SonoraPlayer *)ptr;
    p.running = NO; p.paused = 0; [p.node stop]; [p.engine pause];
}
void sonora_player_gain(void *ptr, float db) { ((__bridge SonoraPlayer *)ptr).gain.globalGain = db; }
void sonora_player_free(void *ptr) {
    @autoreleasepool { SonoraPlayer *p = CFBridgingRelease(ptr); [p.node stop]; [p.engine stop]; }
}
void sonora_reveal(const char *path) {
    @autoreleasepool { [[NSWorkspace sharedWorkspace] activateFileViewerSelectingURLs:@[[NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil]]]; }
}
static void fix_glass(NSView *view) {
    if ([view isKindOfClass:NSVisualEffectView.class]) {
        NSVisualEffectView *glass = (NSVisualEffectView *)view;
        glass.material = NSVisualEffectMaterialUnderWindowBackground;
        glass.blendingMode = NSVisualEffectBlendingModeBehindWindow;
        glass.state = NSVisualEffectStateActive;
    }
    for (NSView *child in view.subviews) fix_glass(child);
}
void sonora_apply_vibrancy(void) {
    for (NSWindow *window in NSApp.windows) {
        window.appearance = [NSAppearance appearanceNamed:NSAppearanceNameDarkAqua];
        fix_glass(window.contentView.superview ?: window.contentView);
    }
}
