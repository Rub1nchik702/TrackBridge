using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Windows.Forms;

namespace TrackBridge {
static class ModernChecks {
 internal static void Draw(Graphics graphics,Rectangle bounds,bool value,bool enabled,bool hover){
  var previous=graphics.Save();graphics.SmoothingMode=SmoothingMode.AntiAlias;
  using(var shape=Surface.Shape(new Rectangle(bounds.X,bounds.Y,bounds.Width-1,bounds.Height-1),4)){
   Color fill=value?(enabled?Color.FromArgb(132,242,187):Color.FromArgb(63,88,77)):Color.FromArgb(32,38,49);
   using(var brush=new SolidBrush(fill))graphics.FillPath(brush,shape);
   using(var pen=new Pen(value?fill:hover?Color.FromArgb(190,206,220):Color.FromArgb(113,128,148),1.3f))graphics.DrawPath(pen,shape);
  }
  if(value){float x=bounds.X,y=bounds.Y,s=bounds.Width/18f;using(var pen=new Pen(Color.FromArgb(16,46,34),2*s)){pen.StartCap=pen.EndCap=LineCap.Round;pen.LineJoin=LineJoin.Round;graphics.DrawLines(pen,new[]{new PointF(x+4*s,y+9*s),new PointF(x+7.5f*s,y+12.5f*s),new PointF(x+14*s,y+5.5f*s)});}}
  graphics.Restore(previous);
 }
}
class ModernCheckBox : CheckBox {
 bool hover;
 public ModernCheckBox(){SetStyle(ControlStyles.UserPaint|ControlStyles.AllPaintingInWmPaint|ControlStyles.OptimizedDoubleBuffer|ControlStyles.SupportsTransparentBackColor,true);BackColor=Color.Transparent;AutoSize=true;Margin=new Padding(0,4,14,4);Cursor=Cursors.Hand;}
 public override Size GetPreferredSize(Size proposedSize){var text=TextRenderer.MeasureText(Text,Font,Size.Empty,TextFormatFlags.NoPadding);return new Size(text.Width+32,Math.Max(26,text.Height+6));}
 protected override void OnPaintBackground(PaintEventArgs e){Control owner=Parent;while(owner!=null && owner.BackColor.A<255)owner=owner.Parent;e.Graphics.Clear(owner==null?Color.FromArgb(17,21,29):owner.BackColor);}
 protected override void OnPaint(PaintEventArgs e){OnPaintBackground(e);int box=Math.Max(18,(int)(18*DeviceDpi/96f));ModernChecks.Draw(e.Graphics,new Rectangle(1,(Height-box)/2,box,box),Checked,Enabled,hover);var text=new Rectangle(box+10,0,Math.Max(0,Width-box-10),Height);TextRenderer.DrawText(e.Graphics,Text,Font,text,Enabled?ForeColor:Color.FromArgb(111,124,143),TextFormatFlags.Left|TextFormatFlags.VerticalCenter|TextFormatFlags.EndEllipsis|TextFormatFlags.NoPadding);if(Focused && ShowFocusCues)ControlPaint.DrawFocusRectangle(e.Graphics,ClientRectangle);}
 protected override void OnMouseEnter(EventArgs e){hover=true;Invalidate();base.OnMouseEnter(e);}
 protected override void OnMouseLeave(EventArgs e){hover=false;Invalidate();base.OnMouseLeave(e);}
 protected override void OnCheckedChanged(EventArgs e){Invalidate();base.OnCheckedChanged(e);}
 protected override void OnEnabledChanged(EventArgs e){Invalidate();base.OnEnabledChanged(e);}
}
static class ThemeArrows {
 internal static void Draw(Graphics graphics,PointF center,bool up,Color color,float halfWidth=4f){var state=graphics.Save();graphics.SmoothingMode=SmoothingMode.AntiAlias;float direction=up?-1:1;using(var pen=new Pen(color,1.6f)){pen.StartCap=pen.EndCap=LineCap.Round;pen.LineJoin=LineJoin.Round;graphics.DrawLines(pen,new[]{new PointF(center.X-halfWidth,center.Y-direction*2),new PointF(center.X,center.Y+direction*2),new PointF(center.X+halfWidth,center.Y-direction*2)});}graphics.Restore(state);}
}
class ThemedNumericUpDown : NumericUpDown {
 readonly System.Collections.Generic.HashSet<Control> styled=new System.Collections.Generic.HashSet<Control>();
 protected override void OnCreateControl(){base.OnCreateControl();foreach(Control child in Controls){if(child is TextBoxBase || !styled.Add(child))continue;var skin=new SpinnerSkin(this,child);child.HandleCreated+=(sender,args)=>skin.Attach();if(child.IsHandleCreated)skin.Attach();}}
 protected override void OnEnabledChanged(EventArgs e){base.OnEnabledChanged(e);foreach(Control child in Controls)child.Invalidate();}
 sealed class SpinnerSkin : NativeWindow {
  readonly ThemedNumericUpDown owner;readonly Control buttons;
  internal SpinnerSkin(ThemedNumericUpDown owner,Control buttons){this.owner=owner;this.buttons=buttons;}
  internal void Attach(){if(Handle!=IntPtr.Zero)ReleaseHandle();AssignHandle(buttons.Handle);}
  protected override void WndProc(ref Message message){base.WndProc(ref message);if(message.Msg!=0x000F && message.Msg!=0x0317 && message.Msg!=0x0318)return;using(var graphics=(message.Msg==0x000F || message.WParam==IntPtr.Zero)?Graphics.FromHwnd(Handle):Graphics.FromHdc(message.WParam)){using(var background=new SolidBrush(owner.BackColor))graphics.FillRectangle(background,buttons.ClientRectangle);Color color=owner.Enabled?owner.ForeColor:Color.FromArgb(111,124,143);float x=(buttons.Width-1)/2f;ThemeArrows.Draw(graphics,new PointF(x,buttons.Height/4f),true,color,3f);ThemeArrows.Draw(graphics,new PointF(x,buttons.Height*3/4f),false,color,3f);}}
 }
}
class ThemedComboBox : ComboBox {
 public ThemedComboBox(){FlatStyle=FlatStyle.Flat;DrawMode=DrawMode.OwnerDrawFixed;DropDownStyle=ComboBoxStyle.DropDownList;ItemHeight=24;BackColor=Color.FromArgb(34,40,51);ForeColor=Color.FromArgb(230,234,241);}
 protected override void WndProc(ref Message message){base.WndProc(ref message);if(!IsHandleCreated || (message.Msg!=0x000F && message.Msg!=0x0317 && message.Msg!=0x0318))return;using(var graphics=(message.Msg==0x000F || message.WParam==IntPtr.Zero)?Graphics.FromHwnd(Handle):Graphics.FromHdc(message.WParam)){
  using(var background=new SolidBrush(BackColor))graphics.FillRectangle(background,ClientRectangle);graphics.SmoothingMode=SmoothingMode.AntiAlias;
  using(var border=new Pen(Color.FromArgb(76,86,103)))graphics.DrawRectangle(border,0,0,Width-1,Height-1);
  int buttonWidth=Math.Max(32,(int)(32*DeviceDpi/96f));var textBounds=new Rectangle(8,0,Math.Max(0,Width-buttonWidth-12),Height);
  TextRenderer.DrawText(graphics,SelectedIndex>=0?GetItemText(SelectedItem):"",Font,textBounds,Enabled?ForeColor:Color.FromArgb(111,124,143),TextFormatFlags.Left|TextFormatFlags.VerticalCenter|TextFormatFlags.EndEllipsis);
  ThemeArrows.Draw(graphics,new PointF(Width-buttonWidth/2f,(Height-1)/2f),false,Enabled?ForeColor:Color.FromArgb(111,124,143));
 }}
 protected override void OnDrawItem(DrawItemEventArgs e){if(e.Bounds.Width<=0 || e.Bounds.Height<=0)return;bool selected=(e.State & DrawItemState.Selected)!=0 && (e.State & DrawItemState.ComboBoxEdit)==0;Color background=selected?Color.FromArgb(43,74,64):BackColor;using(var brush=new SolidBrush(background))e.Graphics.FillRectangle(brush,e.Bounds);string text=e.Index>=0 && e.Index<Items.Count?GetItemText(Items[e.Index]):"";var bounds=e.Bounds;bounds.Inflate(-7,0);TextRenderer.DrawText(e.Graphics,text,Font,bounds,Enabled?ForeColor:Color.FromArgb(111,124,143),TextFormatFlags.Left|TextFormatFlags.VerticalCenter|TextFormatFlags.EndEllipsis);if((e.State & DrawItemState.Focus)!=0 && Focused)ControlPaint.DrawFocusRectangle(e.Graphics,e.Bounds,ForeColor,background);}
}
class PageTransition : Control {
 readonly Bitmap before,after;readonly System.Windows.Forms.Timer timer=new System.Windows.Forms.Timer{Interval=15};readonly System.Diagnostics.Stopwatch elapsed=new System.Diagnostics.Stopwatch();
 [System.Runtime.InteropServices.DllImport("user32.dll")]static extern bool SystemParametersInfo(uint action,uint parameter,out bool enabled,uint flags);
 internal static bool MotionEnabled{get{bool enabled;return !SystemParametersInfo(0x1042,0,out enabled,0) || enabled;}}
 internal PageTransition(Bitmap before,Bitmap after){this.before=before;this.after=after;SetStyle(ControlStyles.UserPaint|ControlStyles.AllPaintingInWmPaint|ControlStyles.OptimizedDoubleBuffer,true);timer.Tick+=(sender,args)=>{if(elapsed.ElapsedMilliseconds>=160){Dispose();return;}Invalidate();};}
 internal void Start(){elapsed.Start();timer.Start();}
 protected override void OnPaint(PaintEventArgs e){float progress=Math.Min(1,elapsed.ElapsedMilliseconds/160f);float opacity=progress*progress*(3-2*progress);e.Graphics.DrawImageUnscaled(before,0,0);using(var attributes=new System.Drawing.Imaging.ImageAttributes()){var matrix=new System.Drawing.Imaging.ColorMatrix();matrix.Matrix33=opacity;attributes.SetColorMatrix(matrix);e.Graphics.DrawImage(after,ClientRectangle,0,0,after.Width,after.Height,GraphicsUnit.Pixel,attributes);}}
 protected override void Dispose(bool disposing){if(disposing){timer.Stop();timer.Dispose();before.Dispose();after.Dispose();}base.Dispose(disposing);}
}
class CountBadge : Label {
 public CountBadge(){SetStyle(ControlStyles.UserPaint|ControlStyles.AllPaintingInWmPaint|ControlStyles.OptimizedDoubleBuffer|ControlStyles.SupportsTransparentBackColor,true);BackColor=Color.Transparent;}
 protected override void OnPaint(PaintEventArgs e){e.Graphics.SmoothingMode=SmoothingMode.AntiAlias;using(var path=Surface.Shape(new Rectangle(0,0,Width-1,Height-1),10))using(var brush=new SolidBrush(Color.FromArgb(39,65,58)))e.Graphics.FillPath(brush,path);TextRenderer.DrawText(e.Graphics,Text,Font,ClientRectangle,Color.FromArgb(132,242,187),TextFormatFlags.HorizontalCenter|TextFormatFlags.VerticalCenter);}
}
}
