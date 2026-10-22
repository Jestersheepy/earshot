# Writes models/speakers_test: a short conversation between three Windows TTS voices, for the speaker tests.
#     powershell -ExecutionPolicy Bypass -File tools/make_speaker_test.ps1
Add-Type -AssemblyName System.Speech
$out = Join-Path $PSScriptRoot "..\models\speakers_test"
New-Item -ItemType Directory -Force $out | Out-Null
$turns = @(
 @("Microsoft David Desktop","Okay, let's get started. I want to go through the roadmap for next quarter and agree on priorities."),
 @("Microsoft Zira Desktop","Sounds good. Before we do, I have an update on the customer interviews from last week."),
 @("Microsoft Hazel Desktop","Can I jump in quickly? The design review moved to Thursday, so we have a bit more time."),
 @("Microsoft David Desktop","Great, that helps. Zira, go ahead with the interviews."),
 @("Microsoft Zira Desktop","We spoke to twelve customers. Nine of them asked for better reporting, and most of them want exports to spreadsheets."),
 @("Microsoft Hazel Desktop","That matches what support is seeing. Exports are the top request in tickets this month."),
 @("Microsoft David Desktop","Then reporting should probably be our first priority. Does anyone disagree?"),
 @("Microsoft Hazel Desktop","No, but we need to be careful about scope. Reporting can grow forever if we let it."),
 @("Microsoft Zira Desktop","Agreed. I can write a short spec that limits it to the three most requested reports."),
 @("Microsoft David Desktop","Perfect. Let's review that spec next week and lock the scope then."),
 @("Microsoft Hazel Desktop","I'll bring the support numbers so we can size it properly."),
 @("Microsoft Zira Desktop","And I'll share the interview notes with everyone after this call.")
)
$fmt = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
$i = 0
foreach ($t in $turns) {
  $s.SelectVoice($t[0]); $s.SetOutputToWaveFile(("{0}\turn{1:D2}_{2}.wav" -f $out, $i, $t[0].Split(' ')[1]), $fmt); $s.Speak($t[1]); $i++
}
$s.SetOutputToNull()
