@echo off
rem harness_mini build launcher for Windows.
rem
rem Usage: build.cmd [options]   (options are forwarded to build.sh, e.g. --no-bundle)
rem
rem The project scripts are written in bash, which is not on the Windows PATH by
rem default, so this launcher locates Git Bash automatically:
rem   1) bash.exe already on PATH
rem   2) derived from git.exe on PATH   <Git>\cmd\git.exe -> <Git>\bin\bash.exe
rem   3) common install locations
rem It also switches the console to UTF-8 (code page 65001) so the Chinese output
rem of build.sh renders correctly, and restores the previous code page on exit.
rem
rem This file is intentionally ASCII-only: cmd.exe reads .cmd files using the OEM
rem code page, so non-ASCII text here would be garbled in a default console.

setlocal
set "HERE=%~dp0"
set "BASH_EXE="

rem 1) bash.exe on PATH
for %%I in (bash.exe) do if not defined BASH_EXE set "BASH_EXE=%%~$PATH:I"
rem 2) derive from git.exe on PATH
for %%I in (git.exe) do if not defined BASH_EXE call :from_git "%%~$PATH:I"
rem 3) common install locations
if not defined BASH_EXE call :try "%ProgramFiles%\Git\bin\bash.exe"
if not defined BASH_EXE call :try "%LOCALAPPDATA%\Programs\Git\bin\bash.exe"
if not defined BASH_EXE call :try "%ProgramFiles(x86)%\Git\bin\bash.exe"

if not defined BASH_EXE (
  echo [ERROR] Git Bash ^(bash.exe^) not found.
  echo.
  echo Install Git for Windows: https://git-scm.com/download/win
  echo or add Git's bin directory to PATH, then retry.
  exit /b 1
)

rem remember current code page, switch to UTF-8 for build.sh output
set "OLDCP="
for /f "tokens=2 delims=:" %%A in ('chcp') do set "OLDCP=%%A"
chcp 65001 >nul

"%BASH_EXE%" "%HERE%build.sh" %*
set "RC=%ERRORLEVEL%"

if defined OLDCP chcp%OLDCP% >nul
exit /b %RC%

:try
rem resolve to a full path first: cmd's "if exist" does not collapse ".." segments
for %%I in ("%~1") do if not defined BASH_EXE if exist "%%~fI" set "BASH_EXE=%%~fI"
exit /b 0

:from_git
rem %~dp1 is git.exe's directory and ends with a backslash.
rem Layout A: <Git>\cmd\git.exe  -> one level up
rem Layout B: <Git>\bin\git.exe  -> same directory
if not defined BASH_EXE call :try "%~dp1..\bin\bash.exe"
if not defined BASH_EXE call :try "%~dp1bash.exe"
exit /b 0
