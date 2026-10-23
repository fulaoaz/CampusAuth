import {createDarkTheme,createLightTheme} from '@fluentui/react-components';
import type {BrandVariants} from '@fluentui/react-components';
import type {Theme} from '@fluentui/react-components';
const campusBrand:BrandVariants={10:'#001e20',20:'#003639',30:'#00484b',40:'#00595d',50:'#006a70',60:'#087e84',70:'#0c9096',80:'#19a2a7',90:'#39b3b6',100:'#5cc2c4',110:'#7bd0d1',120:'#98dcdd',130:'#b5e7e8',140:'#cff0f0',150:'#e4f7f7',160:'#f3fcfc'};
const shared={fontFamilyBase:'"Segoe UI Variable", "Campus Sans", "Microsoft YaHei UI", sans-serif',fontFamilyNumeric:'"Segoe UI Variable", "Campus Sans", sans-serif',fontSizeBase100:'13px',fontSizeBase200:'13px',fontSizeBase300:'15px',fontSizeBase400:'16px',lineHeightBase100:'20px',lineHeightBase200:'20px',lineHeightBase300:'24px',lineHeightBase400:'26px',borderRadiusMedium:'8px',borderRadiusLarge:'12px'};
export const lightTheme={...createLightTheme(campusBrand),...shared,colorBrandBackground:'#087e84',colorBrandBackgroundHover:'#076e74',colorBrandBackgroundPressed:'#055b60',colorBrandForeground1:'#087e84'};
export const darkTheme={...createDarkTheme(campusBrand),...shared,colorBrandBackground:'#0d9196',colorBrandBackgroundHover:'#16a5aa',colorBrandForeground1:'#6dd6d7',colorNeutralBackground1:'#1b2733',colorNeutralBackground1Hover:'#233443',colorNeutralBackground1Pressed:'#293e4e',colorNeutralBackground1Selected:'#253b48',colorNeutralBackground2:'#17232e',colorNeutralBackground3:'#21313e',colorNeutralBackground4:'#253744',colorNeutralBackground5:'#121b24',colorNeutralBackground6:'#17232e',colorNeutralForeground1:'#e3edf4',colorNeutralForeground2:'#bbcad6',colorNeutralForeground3:'#9aafbf',colorNeutralStroke1:'#3a4e5e',colorNeutralStroke2:'#2c3c4a',colorNeutralStrokeAccessible:'#829aaa',colorNeutralBackgroundInverted:'#eef5f8'};
export function withTextSize(theme:Theme,size:number):Theme {
  const scale=size/15;
  return Object.fromEntries(Object.entries(theme).map(([name,value])=>{
    if((name.startsWith('fontSize')||name.startsWith('lineHeight'))&&typeof value==='string'&&/^\d+(\.\d+)?px$/.test(value))return[name,`${Math.round(parseFloat(value)*scale*100)/100}px`];
    return[name,value];
  })) as Theme;
}
