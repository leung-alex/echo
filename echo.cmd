@echo off
setlocal EnableExtensions EnableDelayedExpansion

set "ROOT=%~dp0"
cd /d "%ROOT%" || exit /b 1

for /f "delims=" %%V in ('go env GOVERSION 2^>nul') do set "GO_VERSION=%%V"
if not "%GO_VERSION%"=="go1.26.2" (
  echo echo requires Go 1.26.2; found %GO_VERSION% 1>&2
  exit /b 1
)

set "CACHE_ROOT=%ROOT%target\echo\bootstrap-cache"
set "RUN_ROOT=%ROOT%.local\echo\run"
if not exist "%CACHE_ROOT%" mkdir "%CACHE_ROOT%" || exit /b 1
if not exist "%RUN_ROOT%" mkdir "%RUN_ROOT%" || exit /b 1

:allocate_session
set "SESSION_ROOT=%RUN_ROOT%\session-%RANDOM%-%RANDOM%"
mkdir "%SESSION_ROOT%" >nul 2>nul
if errorlevel 1 goto allocate_session

set "KEY_INPUT=%SESSION_ROOT%\key.txt"
break > "%KEY_INPUT%"
for /f "delims=" %%F in ('git ls-files --cached --others --exclude-standard -- "tools/echo/*.go" "tools/echo/go.mod" "tools/echo/go.sum"') do (
  for /f "delims=" %%H in ('git hash-object "%%F"') do echo %%H>>"%KEY_INPUT%"
)
for /f "delims=" %%K in ('git hash-object "%KEY_INPUT%"') do set "CACHE_KEY=%%K"
del /q "%KEY_INPUT%" >nul 2>nul
if not defined CACHE_KEY (
  echo failed to compute Echo bootstrap cache key 1>&2
  rmdir "%SESSION_ROOT%" >nul 2>nul
  exit /b 1
)

set "CACHED_EXE=%CACHE_ROOT%\echo-%CACHE_KEY%.exe"
if not exist "%CACHED_EXE%" (
  echo [echo bootstrap] build %CACHE_KEY%
  set "BUILD_EXE=%SESSION_ROOT%\echo-build.exe"
  go -C tools\echo build -trimpath -o "!BUILD_EXE!" .
  if errorlevel 1 (
    set "ECHO_EXIT=!ERRORLEVEL!"
    del /q "!BUILD_EXE!" >nul 2>nul
    rmdir "%SESSION_ROOT%" >nul 2>nul
    exit /b !ECHO_EXIT!
  )
  move /y "!BUILD_EXE!" "%CACHED_EXE%" >nul 2>nul
  if errorlevel 1 if not exist "%CACHED_EXE%" (
    del /q "!BUILD_EXE!" >nul 2>nul
    rmdir "%SESSION_ROOT%" >nul 2>nul
    exit /b 1
  )
  del /q "!BUILD_EXE!" >nul 2>nul
)

set "RUN_EXE=%SESSION_ROOT%\echo.exe"
copy /y "%CACHED_EXE%" "%RUN_EXE%" >nul || (
  rmdir "%SESSION_ROOT%" >nul 2>nul
  exit /b 1
)
"%RUN_EXE%" %*
set "ECHO_EXIT=%ERRORLEVEL%"
del /q "%RUN_EXE%" >nul 2>nul
rmdir "%SESSION_ROOT%" >nul 2>nul
exit /b %ECHO_EXIT%
