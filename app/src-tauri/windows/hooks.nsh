;;; 篝火安装包的 NSIS 钩子（Tauri bundle.windows.nsis.installerHooks）。
;;; 卸载时询问是否同时删除 <安装目录>\data（备份仓库、设置、日志、缓存），默认保留；
;;; 提示：远程同步的备份（GitHub、Gitee、WebDAV、网盘、其他文件夹）不会被删除；仓库位置改到 data 以外时，那里的仓库也不会被删除。
;;; 升级（/UPDATE）时不询问、不删除。静默卸载默认保留，加 /DELETEDATA 时删除（安装包测试使用）。

!macro BONFIRE_REPO_LINE
  ; 读 data\config.toml 中的 repo = ...（UTF-8），结果在 $R5（没有改过仓库位置时为空）。
  StrCpy $R5 ""
  ClearErrors
  FileOpen $R6 "$INSTDIR\data\config.toml" r
  ${IfNot} ${Errors}
    System::Alloc 65536
    Pop $R7
    System::Call 'kernel32::ReadFile(p R6, p R7, i 65535, *i 0 R8, p 0)'
    FileClose $R6
    System::Alloc 131072
    Pop $R9
    System::Call 'kernel32::MultiByteToWideChar(i 65001, i 0, p R7, i R8, p R9, i 65535)'
    System::Call 'shlwapi::StrStrW(p R9, w "$\nrepo = ") p .R6'
    ${If} $R6 P<> 0
      IntPtrOp $R6 $R6 + 16
      System::Call '*$R6(&w1024 .R5)'
      ; 截到行尾，去掉引号
      StrCpy $R8 0
      ${Do}
        StrCpy $R6 $R5 1 $R8
        ${If} $R6 == ""
        ${OrIf} $R6 == "$\r"
        ${OrIf} $R6 == "$\n"
          ${Break}
        ${EndIf}
        IntOp $R8 $R8 + 1
      ${Loop}
      StrCpy $R5 $R5 $R8
      StrCpy $R6 $R5 1
      ${If} $R6 == "'"
      ${OrIf} $R6 == '"'
        StrCpy $R5 $R5 "" 1
        StrCpy $R5 $R5 -1
      ${EndIf}
    ${EndIf}
    System::Free $R7
    System::Free $R9
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
  ${AndIf} ${FileExists} "$INSTDIR\data\*.*"
    !insertmacro BONFIRE_REPO_LINE
    StrCpy $R9 "0"
    ${If} ${Silent}
      ClearErrors
      ${GetOptions} $CMDLINE "/DELETEDATA" $R8
      ${IfNot} ${Errors}
        StrCpy $R9 "1"
      ${EndIf}
    ${Else}
      ${If} $LANGUAGE == 2052
        StrCpy $R8 "是否同时删除篝火的数据？$\r$\n$\r$\n$INSTDIR\data 中有：备份仓库（全部存档快照与截图）、设置、日志与缓存。删除后无法恢复。$\r$\n$\r$\n选择“否”会保留数据，重新安装篝火后可以继续使用。$\r$\n$\r$\n请注意：$\r$\n· 已同步到 GitHub、Gitee、WebDAV、网盘或其他文件夹的远程备份不会被删除，需要的话请到对应的网站或文件夹中自行删除；那里的备份可以在重新安装后从远程恢复。"
        ${If} $R5 != ""
          StrCpy $R8 "$R8$\r$\n· 仓库位置设为 $R5 时，那里的仓库不在 data 中，不会被删除。"
        ${EndIf}
      ${Else}
        StrCpy $R8 "Also delete Bonfire's data?$\r$\n$\r$\n$INSTDIR\data holds the backup repository (every save snapshot and screenshot), settings, logs and cache. This can't be undone.$\r$\n$\r$\nChoose No to keep the data and use it again after reinstalling.$\r$\n$\r$\nNote:$\r$\n· Remote backups synced to GitHub, Gitee, WebDAV, a cloud drive or another folder are NOT deleted; remove them on the website or in the folder yourself if needed. They can be restored after reinstalling."
        ${If} $R5 != ""
          StrCpy $R8 "$R8$\r$\n· If the repository location is $R5, it isn't inside data and won't be deleted."
        ${EndIf}
      ${EndIf}
      MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "$R8" IDNO bonfire_keep_data
      ${If} $LANGUAGE == 2052
        MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "确定永久删除 $INSTDIR\data 吗？全部存档备份与截图都会删除。" IDNO bonfire_keep_data
      ${Else}
        MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "Permanently delete $INSTDIR\data? All save backups and screenshots will be deleted." IDNO bonfire_keep_data
      ${EndIf}
      StrCpy $R9 "1"
      bonfire_keep_data:
    ${EndIf}
    ${If} $R9 == "1"
      RMDir /r "$INSTDIR\data"
      RMDir "$INSTDIR"
    ${ElseIfNot} ${Silent}
      ${If} $LANGUAGE == 2052
        MessageBox MB_OK|MB_ICONINFORMATION "已保留数据：$INSTDIR\data$\r$\n$\r$\n远程同步的备份也没有变化。不再需要时，可以手动删除这个文件夹。"
      ${Else}
        MessageBox MB_OK|MB_ICONINFORMATION "Your data was kept: $INSTDIR\data$\r$\n$\r$\nRemote backups are unchanged too. Delete this folder yourself when you no longer need it."
      ${EndIf}
    ${EndIf}
  ${EndIf}
!macroend
