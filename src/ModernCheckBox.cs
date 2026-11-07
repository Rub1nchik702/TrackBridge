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
 protected override void OnPaint(PaintEventArgs e){int box=Math.Max(18,(int)(18*DeviceDpi/96f));ModernChecks.Draw(e.Graphics,new Rectangle(1,(Height-box)/2,box,box),Checked,Enabled,hover);var text=new Rectangle(box+10,0,Math.Max(0,Width-box-10),Height);TextRenderer.DrawText(e.Graphics,Text,Font,text,Enabled?ForeColor:Color.FromArgb(111,124,143),TextFormatFlags.Left|TextFormatFlags.VerticalCenter|TextFormatFlags.EndEllipsis|TextFormatFlags.NoPadding);if(Focused && ShowFocusCues)ControlPaint.DrawFocusRectangle(e.Graphics,ClientRectangle);}
 protected override void OnMouseEnter(EventArgs e){hover=true;Invalidate();base.OnMouseEnter(e);}
 protected override void OnMouseLeave(EventArgs e){hover=false;Invalidate();base.OnMouseLeave(e);}
 protected override void OnCheckedChanged(EventArgs e){Invalidate();base.OnCheckedChanged(e);}
 protected override void OnEnabledChanged(EventArgs e){Invalidate();base.OnEnabledChanged(e);}
}
class CountBadge : Label {
 public CountBadge(){SetStyle(ControlStyles.UserPaint|ControlStyles.AllPaintingInWmPaint|ControlStyles.OptimizedDoubleBuffer|ControlStyles.SupportsTransparentBackColor,true);BackColor=Color.Transparent;}
 protected override void OnPaint(PaintEventArgs e){e.Graphics.SmoothingMode=SmoothingMode.AntiAlias;using(var path=Surface.Shape(new Rectangle(0,0,Width-1,Height-1),10))using(var brush=new SolidBrush(Color.FromArgb(39,65,58)))e.Graphics.FillPath(brush,path);TextRenderer.DrawText(e.Graphics,Text,Font,ClientRectangle,Color.FromArgb(132,242,187),TextFormatFlags.HorizontalCenter|TextFormatFlags.VerticalCenter);}
}
}
