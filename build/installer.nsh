; XenoNet installer customizations, included by electron-builder's NSIS
; template.
;
; Adds a welcome page and an Options page (desktop shortcut, start at
; sign-in), gives the wizard the app's dark look, and handles these switches
; on top of the standard ones:
;
;   Install:    /S              silent
;               /currentuser    per-user, no admin (default)
;               /allusers       all users, Program Files (needs admin)
;               /D=<folder>     install folder (must be last)
;               /NODESKTOP      no desktop shortcut
;               /AUTOSTART      start at sign-in, waiting in the tray (the
;                               default on a fresh install)
;               /NOAUTOSTART    don't
;
;   A reinstall keeps the current desktop shortcut and autostart choices
;   unless a switch says otherwise. Every install puts the xenonet command's
;   folder on the user's Path (see XenoNetPathAdd).
;
;   Uninstall:  /S              silent
;               /PURGE          also remove saved layout, alerts and search index
;                               (%LOCALAPPDATA%\XenoNet) and app settings
;                               (%APPDATA%\XenoNet). Posters and clips in
;                               Pictures and Videos are always kept.

!define MUI_BGCOLOR 05070A
!define MUI_TEXTCOLOR DCE6EE
; "Are you sure you want to quit?" on Cancel, like most installers.
!define MUI_ABORTWARNING
!define MUI_UNABORTWARNING

!include LogicLib.nsh
!include FileFunc.nsh
!include WordFunc.nsh
!include WinMessages.nsh
!include nsDialogs.nsh

!define XENONET_RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
; Named after the app ID, the value Electron reads and writes for the tray
; menu's "Start at sign-in".
!define XENONET_RUN_VALUE "${APP_ID}"

; The xenonet command's folder (core/command.js commandDir), as the user's Path
; holds it: unexpanded, in a REG_EXPAND_SZ value, so it follows the profile.
!define XENONET_PATH_ENTRY "%LOCALAPPDATA%\XenoNet\bin"

; Adds XENONET_PATH_ENTRY to HKCU\<KEY>\Path (KEY is "Environment" for real;
; tests point it elsewhere) unless it's there already, in either form. The
; folder may not exist yet: Desktop's first launch writes xenonet.cmd into it.
; Other entries keep their %VARIABLES% and order. A Path too close to NSIS's
; string limit to read whole is left alone rather than cut short.
!macro XenoNetPathAdd KEY
  Push $R0
  Push $R1
  Push $R2
  ReadRegStr $R0 HKCU "${KEY}" "Path"
  StrLen $R1 $R0
  ${If} $R1 < 8000
    ; Drop trailing separators so the entry isn't added after an empty one.
    ${DoWhile} $R0 != ""
      StrCpy $R1 $R0 1 -1
      ${IfThen} $R1 != ";" ${|} ${ExitDo} ${|}
      StrCpy $R0 $R0 -1
    ${Loop}
    ReadEnvStr $R1 LOCALAPPDATA
    ${WordAdd} "$R0" ";" "+$R1\XenoNet\bin" $R2
    ${If} $R0 == ""
      StrCpy $R2 "${XENONET_PATH_ENTRY}"
    ${ElseIf} $R2 != $R0
      ${WordAdd} "$R0" ";" "+${XENONET_PATH_ENTRY}" $R2
    ${EndIf}
    ${If} $R2 != $R0
      WriteRegExpandStr HKCU "${KEY}" "Path" "$R2"
      ; New terminals started from Explorer pick it up without signing out.
      SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
    ${EndIf}
  ${EndIf}
  Pop $R2
  Pop $R1
  Pop $R0
!macroend

; Takes the entry XenoNetPathAdd wrote back out of HKCU\<KEY>\Path, leaving
; everything else as it was.
!macro XenoNetPathRemove KEY
  Push $R0
  Push $R1
  ReadRegStr $R0 HKCU "${KEY}" "Path"
  ${WordAdd} "$R0" ";" "-${XENONET_PATH_ENTRY}" $R1
  ${If} $R1 != $R0
    ${If} $R1 == ""
      DeleteRegValue HKCU "${KEY}" "Path"
    ${Else}
      WriteRegExpandStr HKCU "${KEY}" "Path" "$R1"
    ${EndIf}
    SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
  ${EndIf}
  Pop $R1
  Pop $R0
!macroend

!ifndef BUILD_UNINSTALLER
  Var XenoNetWantDesktop
  Var XenoNetWantAutostart
  Var XenoNetDesktopBox
  Var XenoNetAutostartBox

  ; Defaults: a fresh install gets a desktop shortcut and starts at sign-in;
  ; a reinstall keeps whatever is there now. Switches override both.
  !macro customInit
    StrCpy $XenoNetWantDesktop 1
    StrCpy $XenoNetWantAutostart 1
    ReadRegStr $R2 SHCTX "${INSTALL_REGISTRY_KEY}" InstallLocation
    ${If} $R2 != ""
      ${IfNot} ${FileExists} "$DESKTOP\${SHORTCUT_NAME}.lnk"
        StrCpy $XenoNetWantDesktop 0
      ${EndIf}
      ReadRegStr $R2 SHCTX "${XENONET_RUN_KEY}" "${XENONET_RUN_VALUE}"
      ${If} $R2 == ""
        StrCpy $XenoNetWantAutostart 0
      ${EndIf}
    ${EndIf}
    ${GetParameters} $R0
    ClearErrors
    ${GetOptions} $R0 "/NODESKTOP" $R1
    ${IfNot} ${Errors}
      StrCpy $XenoNetWantDesktop 0
    ${EndIf}
    ClearErrors
    ${GetOptions} $R0 "/AUTOSTART" $R1
    ${IfNot} ${Errors}
      StrCpy $XenoNetWantAutostart 1
    ${EndIf}
    ClearErrors
    ${GetOptions} $R0 "/NOAUTOSTART" $R1
    ${IfNot} ${Errors}
      StrCpy $XenoNetWantAutostart 0
    ${EndIf}
    ClearErrors
  !macroend

  !macro customWelcomePage
    !insertmacro skipPageIfUpdated
    !define MUI_WELCOMEPAGE_TITLE "Set up XenoNet ${VERSION}"
    !define MUI_WELCOMEPAGE_TEXT "XenoNet is a live console for your projects and the Claude Code and Codex sessions working in them. It shows what's running, tells you when a session needs you or two sessions edit the same file, and lets you open, hand off or inspect any of it from one place.$\r$\n$\r$\nSetup installs it for your account only, without administrator rights, unless you choose otherwise. If XenoNet is running, Setup closes it first.$\r$\n$\r$\nClick Next to continue."
    !insertmacro MUI_PAGE_WELCOME
  !macroend

  ; electron-builder's finish page, plus one fix: a themed checkbox ignores
  ; its text color, so "Run XenoNet" came out black on the dark page.
  ; Dropping the theme on that one control lets SetCtlColors apply.
  !macro customFinishPage
    !define MUI_FINISHPAGE_RUN
    !define MUI_FINISHPAGE_RUN_FUNCTION "XenoNetStartApp"
    !define MUI_PAGE_CUSTOMFUNCTION_SHOW XenoNetFinishShow
    !insertmacro MUI_PAGE_FINISH

    Function XenoNetStartApp
      ${if} ${isUpdated}
        StrCpy $1 "--updated"
      ${else}
        StrCpy $1 ""
      ${endif}
      ${StdUtils.ExecShellAsUser} $0 "$launchLink" "open" "$1"
    FunctionEnd

    Function XenoNetFinishShow
      StrCpy $0 $mui.FinishPage.Run
      System::Call 'uxtheme::SetWindowTheme(p r0, w " ", w " ")'
      SetCtlColors $mui.FinishPage.Run "${MUI_TEXTCOLOR}" "${MUI_BGCOLOR}"
    FunctionEnd
  !macroend

  ; This file is included before MUI2, so the page's functions live inside the
  ; macro, which the template expands after MUI2 is loaded.
  !macro customPageAfterChangeDir
    Page custom XenoNetOptionsShow XenoNetOptionsLeave
    !insertmacro XenoNetOptionsFunctions
  !macroend

  !macro XenoNetOptionsFunctions
  Function XenoNetOptionsShow
    !insertmacro MUI_HEADER_TEXT "Options" "Choose a shortcut and how XenoNet starts."
    nsDialogs::Create 1018
    Pop $0
    ${NSD_CreateCheckbox} 0 8u 100% 12u "Create a &desktop shortcut"
    Pop $XenoNetDesktopBox
    ${If} $XenoNetWantDesktop == 1
      ${NSD_Check} $XenoNetDesktopBox
    ${EndIf}
    ${NSD_CreateCheckbox} 0 26u 100% 12u "&Start XenoNet in the tray when I sign in"
    Pop $XenoNetAutostartBox
    ${If} $XenoNetWantAutostart == 1
      ${NSD_Check} $XenoNetAutostartBox
    ${EndIf}
    ${NSD_CreateLabel} 0 54u 100% 40u "XenoNet reads your projects and session history. It changes nothing unless you ask it to. You can turn starting at sign-in on or off later from the tray menu."
    Pop $0
    nsDialogs::Show
  FunctionEnd

  Function XenoNetOptionsLeave
    ${NSD_GetState} $XenoNetDesktopBox $XenoNetWantDesktop
    ${NSD_GetState} $XenoNetAutostartBox $XenoNetWantAutostart
  FunctionEnd
  !macroend

  ; Runs after the files, the uninstall entry and electron-builder's shortcuts
  ; are in place. On a reinstall electron-builder leaves existing shortcuts
  ; alone, so the desktop option is applied here either way.
  !macro customInstall
    ${If} $XenoNetWantDesktop == 1
      ${IfNot} ${FileExists} "$newDesktopLink"
        CreateShortCut "$newDesktopLink" "$appExe" "" "$appExe" 0 "" "" "${APP_DESCRIPTION}"
        ClearErrors
        WinShell::SetLnkAUMI "$newDesktopLink" "${APP_ID}"
      ${EndIf}
    ${Else}
      Delete "$newDesktopLink"
    ${EndIf}
    System::Call 'Shell32::SHChangeNotify(i 0x8000000, i 0, i 0, i 0)'

    ${If} $XenoNetWantAutostart == 1
      WriteRegStr SHCTX "${XENONET_RUN_KEY}" "${XENONET_RUN_VALUE}" '"$appExe" --hidden'
    ${Else}
      DeleteRegValue SHCTX "${XENONET_RUN_KEY}" "${XENONET_RUN_VALUE}"
    ${EndIf}

    ; The xenonet command lives in the installing user's own profile, even for
    ; an all-users install, so its folder goes on that user's Path.
    !insertmacro XenoNetPathAdd "Environment"

    ; What Apps & features and winget show beyond electron-builder's defaults.
    WriteRegStr SHCTX "${UNINSTALL_REGISTRY_KEY}" InstallLocation "$INSTDIR"
    ${GetTime} "" "L" $R0 $R1 $R2 $R3 $R4 $R5 $R6
    WriteRegStr SHCTX "${UNINSTALL_REGISTRY_KEY}" InstallDate "$R2$R1$R0"

    ; Keep the updater cache for the next differential download.
  !macroend
!endif

; The uninstaller's first page goes straight to removal, so it says what will
; happen and its button says Uninstall rather than Next.
!macro customUnWelcomePage
  !define MUI_WELCOMEPAGE_TITLE "Uninstall XenoNet"
  !define MUI_WELCOMEPAGE_TEXT "This removes XenoNet Desktop. The background service and tools remain installed. If Desktop is running, it will be closed first.$\r$\n$\r$\nYou'll be asked whether to also delete your saved layout, alerts and search index. Posters and clips you saved, and your Claude Code and Codex sessions, are left alone.$\r$\n$\r$\nClick Uninstall to continue."
  !define MUI_PAGE_CUSTOMFUNCTION_SHOW un.XenoNetUnWelcomeShow
  !insertmacro MUI_UNPAGE_WELCOME
  Function un.XenoNetUnWelcomeShow
    GetDlgItem $0 $HWNDPARENT 1
    SendMessage $0 ${WM_SETTEXT} 0 "STR:&Uninstall"
  FunctionEnd
!macroend

; Runs before the files are removed. Updates run the old uninstaller with
; --updated; those keep the sign-in entry and all data.
!macro customUnInstall
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "--updated" $R1
  ${If} ${Errors}
    DeleteRegValue SHCTX "${XENONET_RUN_KEY}" "${XENONET_RUN_VALUE}"
    DeleteRegValue HKCU "${XENONET_RUN_KEY}" "${XENONET_RUN_VALUE}"
    StrCpy $R3 0
    ClearErrors
    ${GetOptions} $R0 "/PURGE" $R1
    ${IfNot} ${Errors}
      StrCpy $R3 1
    ${ElseIfNot} ${Silent}
      MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "Also remove your XenoNet data?$\r$\n$\r$\nThat stops the background service and its terminals, and removes the component files, saved layout, dismissed alerts, transcript search index and app settings. Posters and clips you saved, and your Claude Code and Codex sessions, are not affected either way." /SD IDNO IDNO xenonet_keep_data
      StrCpy $R3 1
      xenonet_keep_data:
    ${EndIf}
    ${If} $R3 == 1
      ReadEnvStr $R4 XDG_STATE_HOME
      ${If} $R4 != ""
        StrCpy $R4 "$R4\xenonet\runtime\launcher"
      ${Else}
        ReadEnvStr $R4 LOCALAPPDATA
        StrCpy $R4 "$R4\XenoNet\runtime\launcher"
      ${EndIf}
      ; XenoNet's hooks and MCP servers in Claude Code and Codex run the
      ; launcher about to be deleted: take them out first, so no agent's
      ; hook is left pointing at nothing. The launcher is the xenonet binary;
      ; an earlier XenoNet's was a node of its own and cli\xenonet.js.
      ${If} ${FileExists} "$R4\xenonet.exe"
        ExecWait '"$R4\xenonet.exe" disconnect --yes --port 47840' $R2
        ExecWait '"$R4\xenonet.exe" service stop --force' $R2
      ${ElseIf} ${FileExists} "$R4\node.exe"
        ExecWait '"$R4\node.exe" "$R4\cli\xenonet.js" disconnect --yes --port 47840' $R2
        ExecWait '"$R4\node.exe" "$R4\cli\xenonet.js" service stop --force' $R2
      ${EndIf}
      ; The same places server/config.js picks: XDG dirs when set, else
      ; %LOCALAPPDATA%. %APPDATA% holds Electron's settings.
      ReadEnvStr $R4 XDG_STATE_HOME
      ${If} $R4 != ""
        RMDir /r "$R4\xenonet"
      ${EndIf}
      ReadEnvStr $R4 XDG_CACHE_HOME
      ${If} $R4 != ""
        RMDir /r "$R4\xenonet"
      ${EndIf}
      ReadEnvStr $R4 LOCALAPPDATA
      RMDir /r "$R4\XenoNet"
      ReadEnvStr $R4 APPDATA
      RMDir /r "$R4\XenoNet"
      ; The xenonet command went with %LOCALAPPDATA%\XenoNet. Kept data keeps the
      ; command working (it runs the remaining Service and Tools), and its
      ; Path entry with it.
      !insertmacro XenoNetPathRemove "Environment"
    ${EndIf}
    ; Setup keeps a copy of itself here for electron-builder's updater.
    ReadEnvStr $R4 LOCALAPPDATA
    RMDir /r "$R4\xenonet-updater"
  ${EndIf}
!macroend
