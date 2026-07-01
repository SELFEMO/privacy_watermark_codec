!macro PWC_PICK_NON_SYSTEM_INSTALL_DIR
  StrCpy $R0 "$INSTDIR" 3
  StrCmp $R0 "C:\" 0 pwc_done

  ; 中文：如果默认路径落在系统盘，就按 D-Z 顺序选择第一个可用盘符，减少用户数据进入系统盘的概率。
  ; English: If the default path is on the system drive, pick the first available D-Z drive to reduce system-drive data usage.
  IfFileExists "D:\*.*" 0 pwc_try_e
    StrCpy $INSTDIR "D:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_e:
  IfFileExists "E:\*.*" 0 pwc_try_f
    StrCpy $INSTDIR "E:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_f:
  IfFileExists "F:\*.*" 0 pwc_try_g
    StrCpy $INSTDIR "F:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_g:
  IfFileExists "G:\*.*" 0 pwc_try_h
    StrCpy $INSTDIR "G:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_h:
  IfFileExists "H:\*.*" 0 pwc_try_i
    StrCpy $INSTDIR "H:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_i:
  IfFileExists "I:\*.*" 0 pwc_try_j
    StrCpy $INSTDIR "I:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_j:
  IfFileExists "J:\*.*" 0 pwc_try_k
    StrCpy $INSTDIR "J:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_k:
  IfFileExists "K:\*.*" 0 pwc_try_l
    StrCpy $INSTDIR "K:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_l:
  IfFileExists "L:\*.*" 0 pwc_try_m
    StrCpy $INSTDIR "L:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_m:
  IfFileExists "M:\*.*" 0 pwc_try_n
    StrCpy $INSTDIR "M:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_n:
  IfFileExists "N:\*.*" 0 pwc_try_o
    StrCpy $INSTDIR "N:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_o:
  IfFileExists "O:\*.*" 0 pwc_try_p
    StrCpy $INSTDIR "O:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_p:
  IfFileExists "P:\*.*" 0 pwc_try_q
    StrCpy $INSTDIR "P:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_q:
  IfFileExists "Q:\*.*" 0 pwc_try_r
    StrCpy $INSTDIR "Q:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_r:
  IfFileExists "R:\*.*" 0 pwc_try_s
    StrCpy $INSTDIR "R:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_s:
  IfFileExists "S:\*.*" 0 pwc_try_t
    StrCpy $INSTDIR "S:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_t:
  IfFileExists "T:\*.*" 0 pwc_try_u
    StrCpy $INSTDIR "T:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_u:
  IfFileExists "U:\*.*" 0 pwc_try_v
    StrCpy $INSTDIR "U:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_v:
  IfFileExists "V:\*.*" 0 pwc_try_w
    StrCpy $INSTDIR "V:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_w:
  IfFileExists "W:\*.*" 0 pwc_try_x
    StrCpy $INSTDIR "W:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_x:
  IfFileExists "X:\*.*" 0 pwc_try_y
    StrCpy $INSTDIR "X:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_y:
  IfFileExists "Y:\*.*" 0 pwc_try_z
    StrCpy $INSTDIR "Y:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done
  pwc_try_z:
  IfFileExists "Z:\*.*" 0 pwc_done
    StrCpy $INSTDIR "Z:\Applications\PrivacyWatermarkCodec"
    Goto pwc_done

  pwc_done:
!macroend

!macro PWC_REGISTER_IMAGE_CONTEXT_MENU
  SetRegView 64
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec" "MUIVerb" "Privacy Watermark Codec"
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec" "Icon" "$INSTDIR\privacy-watermark-codec.exe,0"
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec" "SubCommands" ""
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec" "MultiSelectModel" "Player"

  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec\shell\encode" "MUIVerb" "编码隐私水印 / Encode privacy watermark"
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec\shell\encode" "Icon" "$INSTDIR\privacy-watermark-codec.exe,0"
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec\shell\encode" "MultiSelectModel" "Player"
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec\shell\encode\command" "" '"$INSTDIR\privacy-watermark-codec.exe" --pwc-action encode --files "%1"'

  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec\shell\decode" "MUIVerb" "检查隐私水印 / Decode and inspect"
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec\shell\decode" "Icon" "$INSTDIR\privacy-watermark-codec.exe,0"
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec\shell\decode" "MultiSelectModel" "Player"
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec\shell\decode\command" "" '"$INSTDIR\privacy-watermark-codec.exe" --pwc-action decode --files "%1"'

  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec\shell\scan" "MUIVerb" "无密钥扫描 / Keyless scan"
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec\shell\scan" "Icon" "$INSTDIR\privacy-watermark-codec.exe,0"
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec\shell\scan" "MultiSelectModel" "Player"
  WriteRegStr HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec\shell\scan\command" "" '"$INSTDIR\privacy-watermark-codec.exe" --pwc-action scan --files "%1"'
!macroend

!macro PWC_UNREGISTER_IMAGE_CONTEXT_MENU
  SetRegView 64
  DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\image\shell\PrivacyWatermarkCodec"
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro PWC_PICK_NON_SYSTEM_INSTALL_DIR
  CreateDirectory "$INSTDIR\PrivacyWatermarkCodecData"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro PWC_REGISTER_IMAGE_CONTEXT_MENU
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  !insertmacro PWC_UNREGISTER_IMAGE_CONTEXT_MENU
  RMDir /r "$INSTDIR\PrivacyWatermarkCodecData"
!macroend
