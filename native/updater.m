// Sonora's bridge to Sparkle 2 and the standard About panel.
//
// Sparkle.framework is loaded at runtime from Contents/Frameworks with
// NSBundle and reached through NSClassFromString, so Sonora has no link-time
// dependency on it: `cargo run`, tests and source builds without the
// framework simply report the updater as unavailable. The few Sparkle
// methods used are declared below as protocols, copied from the official
// Sparkle 2.10 headers (SPUStandardUpdaterController.h, SPUUpdater.h).
//
// Sparkle's own signature verification (SUPublicEDKey, code signing) is
// never configured or bypassed here; the standard controller and its
// standard user interface do all downloading, verification and installing.
//
// Everything in this file must run on the main thread.

#import <AppKit/AppKit.h>
#include <string.h>

@protocol SonoraSparkleUpdater <NSObject>
- (BOOL)startUpdater:(NSError *__autoreleasing *)error;
@property (nonatomic) BOOL automaticallyChecksForUpdates;
@end

@protocol SonoraSparkleController <NSObject>
- (instancetype)initWithStartingUpdater:(BOOL)startUpdater
                        updaterDelegate:(id)updaterDelegate
                     userDriverDelegate:(id)userDriverDelegate;
@property (nonatomic, readonly) id<SonoraSparkleUpdater> updater;
- (void)checkForUpdates:(id)sender;
@end

// Retained for the life of the app once the updater has started.
static id<SonoraSparkleController> controller;

static void copy_text(NSString *text, char *out, size_t cap) {
    if (!out || cap == 0) return;
    const char *utf8 = text.UTF8String ?: "";
    strncpy(out, utf8, cap - 1);
    out[cap - 1] = 0;
}

// 1 when running from a real .app bundle (not `cargo run`).
int sonora_app_is_bundled(void) {
    NSBundle *main = NSBundle.mainBundle;
    return main.bundleIdentifier.length > 0 && [main.bundlePath.pathExtension isEqualToString:@"app"];
}

// Copies a string value from the main bundle's Info.plist; 0 when missing.
int sonora_bundle_string(const char *key, char *out, size_t cap) {
    @autoreleasepool {
        id value = [NSBundle.mainBundle objectForInfoDictionaryKey:@(key)];
        if (![value isKindOfClass:NSString.class]) return 0;
        copy_text(value, out, cap);
        return 1;
    }
}

// Loads Sparkle and starts the standard updater. 0 on success (or when
// already started); otherwise -1 with a reason in `err`.
int sonora_updater_start(char *err, size_t cap) {
    @autoreleasepool {
        if (controller) return 0;
        NSString *frameworks = NSBundle.mainBundle.privateFrameworksPath;
        NSString *path = [frameworks stringByAppendingPathComponent:@"Sparkle.framework"];
        NSBundle *sparkle = path ? [NSBundle bundleWithPath:path] : nil;
        if (!sparkle) {
            copy_text(@"Sparkle.framework is not bundled with this build.", err, cap);
            return -1;
        }
        NSError *error = nil;
        if (!sparkle.loaded && ![sparkle loadAndReturnError:&error]) {
            copy_text(error.localizedDescription ?: @"Sparkle.framework could not be loaded.", err, cap);
            return -1;
        }
        Class cls = NSClassFromString(@"SPUStandardUpdaterController");
        SEL init = @selector(initWithStartingUpdater:updaterDelegate:userDriverDelegate:);
        if (!cls || ![cls instancesRespondToSelector:init]) {
            copy_text(@"This Sparkle.framework has no SPUStandardUpdaterController.", err, cap);
            return -1;
        }
        // Start explicitly so a misconfiguration comes back as an error
        // here instead of the controller's "contact the developer" alert.
        id<SonoraSparkleController> candidate =
            [(id<SonoraSparkleController>)[cls alloc] initWithStartingUpdater:NO
                                                              updaterDelegate:nil
                                                           userDriverDelegate:nil];
        id<SonoraSparkleUpdater> updater = candidate.updater;
        if (!updater) {
            copy_text(@"Sparkle did not create an updater.", err, cap);
            return -1;
        }
        if (![updater startUpdater:&error]) {
            copy_text(error.localizedDescription ?: @"Sparkle could not start.", err, cap);
            return -1;
        }
        controller = candidate;
        return 0;
    }
}

// The standard "Check for Updates…" action: Sparkle shows its own window.
void sonora_updater_check(void) {
    [controller checkForUpdates:nil];
}

int sonora_updater_automatic(void) {
    return controller && controller.updater.automaticallyChecksForUpdates;
}

void sonora_updater_set_automatic(int on) {
    controller.updater.automaticallyChecksForUpdates = on != 0;
}

// The standard About panel, with a short credits line linking the source.
// `version` is used only when the bundle has no CFBundleShortVersionString
// (unbundled development runs).
void sonora_show_about(const char *version, const char *credits, const char *link) {
    @autoreleasepool {
        NSMutableDictionary *options = [NSMutableDictionary dictionary];
        options[NSAboutPanelOptionApplicationName] = @"Sonora";
        if (![NSBundle.mainBundle objectForInfoDictionaryKey:@"CFBundleShortVersionString"]) {
            options[NSAboutPanelOptionApplicationVersion] = @(version);
            options[NSAboutPanelOptionVersion] = @"";
        }
        NSMutableParagraphStyle *centered = [NSMutableParagraphStyle new];
        centered.alignment = NSTextAlignmentCenter;
        NSDictionary *base = @{
            NSFontAttributeName : [NSFont systemFontOfSize:NSFont.smallSystemFontSize],
            NSForegroundColorAttributeName : NSColor.secondaryLabelColor,
            NSParagraphStyleAttributeName : centered,
        };
        NSString *text = @(credits);
        NSMutableAttributedString *body = [[NSMutableAttributedString alloc] initWithString:text attributes:base];
        NSString *url = @(link);
        NSRange range = [text rangeOfString:[url stringByReplacingOccurrencesOfString:@"https://" withString:@""]];
        if (range.location != NSNotFound) {
            [body addAttribute:NSLinkAttributeName value:[NSURL URLWithString:url] range:range];
        }
        options[NSAboutPanelOptionCredits] = body;
        [NSApp orderFrontStandardAboutPanelWithOptions:options];
    }
}
