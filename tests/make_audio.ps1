# Generates test WAV files with Windows SAPI TTS (16 kHz, 16-bit, mono).
$dir = Join-Path $PSScriptRoot "audio"
Add-Type -AssemblyName System.Speech
$clips = [ordered]@{
  "01_user_example" = "Mm, lets do this first and then I don't know, put these things here to to mhm be able to have good"
  "02_github_env"   = "go to github dot com slash my repo and um open the dot env file"
  "03_email"        = "uh send it to john at gmail dot com"
  "04_question"     = "um what is the capital of France"
  "05_correction"   = "so the meeting is at 5, no wait, at 6 on Friday, and you know it should take about five hundred megabytes of disk space"
  "06_list"         = "uh we need to buy milk, eggs, and bread, new line also call the dentist"
  "07_tech"         = "so I use type script and post gress q l with kubernetes, um, on my mac os machine with a g p u"
  "08_command"      = "write me an email to my boss saying I will be late tomorrow"
  "09_spanish"      = "eh bueno o sea necesito que me envíes el el informe de ventas mañana por la mañana"
  "10_question2"    = "um can you explain how a neural network works"
  "11_command2"     = "uh write a python function that that reverses a string"
  "12_spanish_q"    = "eh cuál es la capital de España"
}
$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
$synth.SelectVoice("Microsoft Zira Desktop")
$fmt = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
foreach ($name in $clips.Keys) {
  $synth.SetOutputToWaveFile((Join-Path $dir "$name.wav"), $fmt)
  $synth.Speak($clips[$name])
  $synth.SetOutputToNull()
}
$synth.Dispose()
