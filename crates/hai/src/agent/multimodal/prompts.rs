/// Transcription template for images.
const IMAGE_STRUCTURE: &str = "You will see an image. Transcribe your observations into a text description — the description will be the only visual input for a text model that cannot understand images, helping it \"see\" the picture. Output according to the following template:\n\n\
Recognition: what this is (an identification conclusion drawn from the visual cues; software/interface, scene, object, document type; if it cannot be made out write \"uncertain\" + the most likely category)\n\
Basis: the cue the judgment rests on (title-bar text/logo/layout features, etc.; one sentence)\n\
Overview: the overall content and atmosphere in one or two sentences\n\
Layout: a rough ASCII sketch of the framing (letters/symbols/lines showing where elements sit and how large they are), or a verbal description of position (left/center/right, front/back, near/far)\n\
Details:\n\
- Subject: what each part of the frame is and where it sits\n\
- People: expressions/actions/postures/interactions (omit if none)\n\
- Text: quote the original verbatim (omit if none)\n\
- Lighting/composition: light sources/tone/key compositional points (omit if none)\n\
- Other: details worth noting\n\n\
Constraints: transcribe faithfully what the frame shows; be complete in detail and cover the main content and important information of the frame — whoever reads this description cannot see the original image.";

/// Transcription template for videos.
const VIDEO_STRUCTURE: &str = "You will see a video. Transcribe your observations into a text description — the description will be the only visual input for a text model that cannot understand images, helping it \"see\" the footage. Output according to the following template:\n\n\
Recognition: the video type and what the content is (screen recording/live footage/animation/presentation, etc.; if it cannot be made out write \"uncertain\" + the most likely category)\n\
Basis: the cue the judgment rests on (visual features, etc.; one sentence)\n\
Overview: the main scenes and people, and the overall atmosphere, in one or two sentences\n\
Timeline: describe the events and key actions in chronological segments\n\
Details:\n\
- People: expressions/actions/interactions (omit if none)\n\
- Objects/scene: environment features and changes of state (omit if none)\n\
- Camera: cuts/camera movement/shot size (omit if none)\n\
- Text/dialogue: quote verbatim (omit if none)\n\
- Other: details worth noting\n\n\
Constraints: transcribe faithfully what the frame shows; the timeline must cover the main events completely — whoever reads this description cannot see the original video.";

/// Transcription template for audio.
const AUDIO_STRUCTURE: &str = "You will hear an audio clip. Transcribe what you hear into a text description — the description will be the only auditory input for a text model that cannot understand audio, helping it \"hear\" the content. Output according to the following template:\n\n\
Recognition: the audio type and what the content is (voice message/meeting recording/music/podcast, etc.; if it cannot be made out write \"uncertain\" + the most likely category)\n\
Basis: the cue the judgment rests on (audible features, etc.; one sentence)\n\
Overview: the topic, the likely setting and the overall atmosphere in one or two sentences\n\
Transcription: transcribe the spoken content word for word in chronological order (distinguish the speakers in a multi-person conversation)\n\
Details:\n\
- Speakers: identification and tone/emotion/attitude (omit if none)\n\
- Background sound: environmental information (omit if none)\n\
- Other: details worth noting\n\n\
Constraints: transcribe faithfully what you hear; the transcription must be complete — whoever reads this description cannot hear the original audio.";

pub(super) const IMAGE_JUDGMENT_PROMPT: &str = "You will see an image. Answer the analysis instruction below directly, giving a conclusion focused on that instruction based on what the frame shows:";
pub(super) const VIDEO_JUDGMENT_PROMPT: &str = "You will see a video. Answer the analysis instruction below directly, giving a conclusion focused on that instruction based on what the frame shows:";
pub(super) const AUDIO_JUDGMENT_PROMPT: &str = "You will hear an audio clip. Answer the analysis instruction below directly, giving a conclusion focused on that instruction based on what you hear:";

pub(super) const DEFAULT_IMAGE_PROMPT: &str = IMAGE_STRUCTURE;
pub(super) const DEFAULT_VIDEO_PROMPT: &str = VIDEO_STRUCTURE;
pub(super) const DEFAULT_AUDIO_PROMPT: &str = AUDIO_STRUCTURE;
pub(super) const DEFAULT_OCR_PROMPT: &str =
    "Extract all the text from this image and return it verbatim.";

pub(super) fn build_prompt(judgment: &str, default_prompt: &str, focus: Option<&str>) -> String {
    match focus {
        Some(f) => format!("{judgment}\n{f}"),
        None => default_prompt.to_string(),
    }
}
