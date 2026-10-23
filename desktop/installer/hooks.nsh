; CampusAuth hooks are shared by the standard and complete offline installers.
; The offline variant uses Tauri's runtime detection. The standard variant uses
; `skip` and this hook to ask before downloading anything on an offline machine.

!macro NSIS_HOOK_PREINSTALL
  ; Tauri serializes WebviewInstallMode::Skip as an empty NSIS mode string.
  !if "${INSTALLWEBVIEW2MODE}" == ""
    ${If} ${RunningX64}
      ReadRegStr $4 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
    ${Else}
      ReadRegStr $4 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
    ${EndIf}
    ${If} $4 == ""
      ReadRegStr $4 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
    ${EndIf}
    ${If} $4 == ""
      IfSilent CampusAuthMissingRuntime
      MessageBox MB_YESNO|MB_DEFBUTTON2|MB_ICONQUESTION "本机尚未安装 Microsoft WebView2 运行时。$\r$\n$\r$\n这是 CampusAuth 标准版，未内置该组件。是否现在联网下载并安装微软官方运行时？$\r$\n$\r$\n如果当前无法联网，请选择“否”，改用 CampusAuth 完整离线安装包。" IDYES CampusAuthDownloadRuntime
      CampusAuthMissingRuntime:
        Abort "缺少 WebView2。请使用 CampusAuth 完整离线安装包，或联网后重新运行标准版安装程序。"
      CampusAuthDownloadRuntime:
        DetailPrint "正在联网下载 Microsoft 官方 WebView2 引导程序..."
        NSISdl::download "https://go.microsoft.com/fwlink/p/?LinkId=2124703" "$TEMP\CampusAuth-WebView2Setup.exe"
        Pop $0
        ${If} $0 != "success"
          Abort "WebView2 下载失败：$0。请改用 CampusAuth 完整离线安装包。"
        ${EndIf}
        ExecWait '"$TEMP\CampusAuth-WebView2Setup.exe" /silent /install' $0
        Delete "$TEMP\CampusAuth-WebView2Setup.exe"
        ${If} $0 != 0
          Abort "WebView2 安装未完成（错误代码 $0）。请改用完整离线安装包，或检查系统安装权限。"
        ${EndIf}
    ${Else}
      DetailPrint "已检测到 WebView2 $4，跳过运行时安装。"
    ${EndIf}
  !endif
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro CheckIfAppIsRunning "$INSTDIR\${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  ${If} $UpdateMode != 1
  ; This command removes only CampusAuth's own autostart registration and power
  ; schedules. An existing system power plan triggers the Windows UAC prompt.
  nsExec::ExecToStack '"$INSTDIR\CampusAuth.exe" --uninstall-cleanup --remove-power-plan'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_OK|MB_ICONEXCLAMATION "CampusAuth 的后台计划未能清理。请允许管理员授权后重试卸载，以免保留定时电源任务。"
    Abort
  ${EndIf}
  IfSilent CampusAuthKeepData
  MessageBox MB_YESNO|MB_DEFBUTTON2|MB_ICONQUESTION "是否同时删除本用户的 CampusAuth 账号、密码、设置和日志？$\r$\n$\r$\n选择“否”会保留数据，方便以后重新安装。" IDNO CampusAuthKeepData
  nsExec::ExecToStack '"$INSTDIR\CampusAuth.exe" --uninstall-cleanup --remove-user-data'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_OK|MB_ICONEXCLAMATION "应用数据未能删除，已保留。可以在卸载后检查用户数据目录。"
  ${EndIf}
  CampusAuthKeepData:
  ${EndIf}
!macroend
