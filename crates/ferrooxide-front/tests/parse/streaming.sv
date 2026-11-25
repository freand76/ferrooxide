module t;
  typedef logic [3:0] nib;
  logic [15:0] a, x;
  logic [7:0] p, q;
  initial begin
    x = {<<8{a}};
    x = {<<{a}};
    x = {>>{a, p}};
    x = {<<byte{a}};
    x = {<<nib{a}};
    x = {<<(2+2){a}};
    {<<8{p, q}} = a;
    {>>{p, q}} <= a;
  end
endmodule
