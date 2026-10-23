import {useState} from 'react';
import type {ReactNode} from 'react';
import {Badge, Button, Combobox, Dialog, DialogActions, DialogBody, DialogContent, DialogSurface, DialogTitle, Field, Input, Option, Spinner, Switch, Tooltip} from '@fluentui/react-components';
import {CheckmarkCircle20Regular, Dismiss20Regular, QuestionCircle20Regular, Warning20Regular} from '@fluentui/react-icons';

export function IconButton({label,icon,onClick,disabled=false,className=''}:{label:string;icon:ReactNode;onClick?:()=>void;disabled?:boolean;className?:string}) {
  return <Tooltip content={label} relationship="label"><Button aria-label={label} appearance="subtle" icon={<>{icon}</>} onClick={onClick} disabled={disabled} className={'icon-button '+className}/></Tooltip>;
}
export function Panel({title,description,actions,children,className=''}:{title?:string;description?:string;actions?:ReactNode;children:ReactNode;className?:string}) {
  return <section className={'panel '+className}>{title&&<div className="panel-heading"><div><h2>{title}</h2>{description&&<p>{description}</p>}</div>{actions}</div>}{children}</section>;
}
export function StatusBadge({state,yes='已连接',no='未连接',unknown='未检测'}:{state:boolean|null|undefined;yes?:string;no?:string;unknown?:string}) {
  return <Badge appearance="tint" color={state===true?'success':state===false?'warning':'informative'} icon={state===true?<CheckmarkCircle20Regular/>:state===false?<Warning20Regular/>:<QuestionCircle20Regular/>}>{state===true?yes:state===false?no:unknown}</Badge>;
}
export function SearchSelect({label,value,options,onChange,placeholder='搜索或选择…',disabled=false,freeform=false}:{label?:string;value:string;options:{value:string;label:string;description?:string}[];onChange:(value:string)=>void;placeholder?:string;disabled?:boolean;freeform?:boolean}) {
  const [query,setQuery]=useState<string|null>(null);
  const selected=options.find(option=>option.value===value);
  const shown=query===null?(selected?.label||value):query;
  const matches=options.filter(option=>query===null||option.label.toLowerCase().includes(query.toLowerCase())||option.value.toLowerCase().includes(query.toLowerCase()));
  const control=<Combobox aria-label={label||placeholder} selectedOptions={value?[value]:[]} value={shown} placeholder={placeholder} disabled={disabled} freeform={freeform} onOpenChange={(_,data)=>{if(!data.open)setQuery(null)}} onChange={event=>setQuery(event.target.value)} onOptionSelect={(_,data)=>{onChange(data.optionValue||'');setQuery(null)}} onBlur={()=>{if(freeform&&query!==null&&query.trim())onChange(query.trim());setQuery(null)}} className="search-select" expandIcon={undefined}>
    {matches.map(option=><Option key={option.value} value={option.value} text={option.label}>{option.label}{option.description&&<span className="option-description">{option.description}</span>}</Option>)}
    {!matches.length&&<Option disabled value="__none">未找到匹配选项</Option>}
  </Combobox>;
  return label?<Field label={label}>{control}</Field>:control;
}
export function SettingRow({title,description,children}:{title:string;description?:string;children:ReactNode}) {
  return <div className="setting-row"><div className="setting-copy"><h3>{title}</h3>{description&&<p>{description}</p>}</div><div className="setting-control">{children}</div></div>;
}
export function ToggleRow({title,description,checked,onChange,disabled=false}:{title:string;description?:string;checked:boolean;onChange:(value:boolean)=>void;disabled?:boolean}) {
  return <SettingRow title={title} description={description}><Switch aria-label={title} checked={checked} onChange={(_,data)=>onChange(data.checked)} disabled={disabled}/></SettingRow>;
}
export function Modal({title,description,open,onClose,children,actions,wide=false}:{title:string;description?:string;open:boolean;onClose:()=>void;children:ReactNode;actions?:ReactNode;wide?:boolean}) {
  return <Dialog open={open} onOpenChange={(_,data)=>{if(!data.open)onClose()}}><DialogSurface className={'app-dialog '+(wide?'wide':'')}><DialogBody><DialogTitle action={<IconButton label="关闭弹窗" icon={<Dismiss20Regular/>} onClick={onClose}/>}>{title}</DialogTitle><DialogContent>{description&&<p className="dialog-description">{description}</p>}{children}</DialogContent>{actions&&<DialogActions>{actions}</DialogActions>}</DialogBody></DialogSurface></Dialog>;
}
export function ConfirmDialog({title,description,action='确认',onConfirm,onClose,busy=false,danger=false}:{title:string;description:string;action?:string;onConfirm:()=>void;onClose:()=>void;busy?:boolean;danger?:boolean}) {
  return <Modal title={title} open description={description} onClose={onClose} actions={<><Button onClick={onClose}>取消</Button><Button appearance="primary" className={danger?'danger-action':''} onClick={onConfirm} disabled={busy}>{busy?<Spinner size="tiny"/>:action}</Button></>}><div/></Modal>;
}
export function EmptyState({icon,title,description,action}:{icon:ReactNode;title:string;description:string;action?:ReactNode}) {
  return <div className="empty-state"><div className="empty-icon">{icon}</div><h3>{title}</h3><p>{description}</p>{action}</div>;
}
export function PasswordInput({value,onChange,placeholder,required=false}:{value:string;onChange:(value:string)=>void;placeholder?:string;required?:boolean}) {
  const [shown,setShown]=useState(false);
  return <Input type={shown?'text':'password'} value={value} onChange={(_,data)=>onChange(data.value)} placeholder={placeholder} autoComplete="off" required={required} contentAfter={<Button appearance="transparent" size="small" onClick={()=>setShown(!shown)} aria-label={shown?'隐藏密码':'显示密码'}>{shown?'隐藏':'显示'}</Button>}/>;
}
