using System;
using System.Drawing;
using System.Drawing.Drawing2D;
namespace TrackBridge {
public static class Brand {
 public static void Draw(Graphics g,Rectangle bounds){
  var state=g.Save();g.SmoothingMode=SmoothingMode.AntiAlias;
  float scale=Math.Min(bounds.Width,bounds.Height)/100f;g.TranslateTransform(bounds.X+(bounds.Width-100*scale)/2,bounds.Y+(bounds.Height-100*scale)/2);g.ScaleTransform(scale,scale);
  using(var p=new Pen(Color.FromArgb(132,242,187),3.4f)){
   p.StartCap=LineCap.Round;p.EndCap=LineCap.Round;p.LineJoin=LineJoin.Round;
   g.DrawLine(p,12,68,12,48);g.DrawLine(p,88,68,88,48);g.DrawLine(p,27,57,27,25);g.DrawLine(p,73,57,73,25);
   using(var curve=new GraphicsPath()){curve.AddBezier(4,53,16,49,25,39,27,25);curve.AddBezier(27,25,33,45,67,45,73,25);curve.AddBezier(73,25,75,39,84,49,96,53);g.DrawPath(p,curve);}
   g.DrawLine(p,4,69,28,61);g.DrawLine(p,72,61,96,69);
   for(int i=0;i<5;i++){int[] heights={12,23,32,23,12};g.DrawLine(p,34+i*8,66-heights[i]/2,34+i*8,66+heights[i]/2);}
  }g.Restore(state);
 }
}
}
