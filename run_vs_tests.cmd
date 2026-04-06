@echo off
call "C:\Program Files\Microsoft Visual Studio\18\Insiders\Common7\Tools\VsDevCmd.bat" -arch=x64 -host_arch=x64
cd /d D:\code2026\pdfXml
cargo test stamp -- --nocapture
