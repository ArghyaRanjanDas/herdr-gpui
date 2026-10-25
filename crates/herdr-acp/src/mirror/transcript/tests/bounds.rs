use super::*;

use crate::session::entry_size;

#[test]
fn a_mirrored_transcript_keeps_no_more_than_a_live_chat() {
    let input = "x".repeat(TEXT);
    let records = (0..MAX_TRANSCRIPT_TEXT / TEXT + 4)
        .map(|index| {
            json!({"type":"assistant","uuid":format!("a{index}"),"message":{"content":[
                {"type":"tool_use","id":format!("t{index}"),"name":"Read","input":input}
            ]}})
        })
        .collect::<Vec<_>>();
    let transcript = parse(TranscriptAgent::Claude, &lines(&records), 0);
    assert!(transcript.truncated);
    assert!(transcript.entries.len() < records.len());
    assert_eq!(keep_from(&transcript.entries), 0);
    let kept: usize = transcript.entries.iter().map(entry_size).sum();
    assert!(kept <= MAX_TRANSCRIPT_TEXT, "{kept}");
}
