// expected
// Streaming concatenations (IEEE 1800-2017 11.4.14): `<<` reverses blocks of the slice size
// taken from the right end (`{<<4{6'b110101}}` is 6'b010111, the standard's example), a wider
// target gets the stream left-justified; as a target the value's leftmost bits unpack.
module t;
  logic [15:0] a = 16'hABCD;
  logic [5:0] b = 6'b110101;
  logic [7:0] c = 8'b1100_0001;
  logic [15:0] x;
  logic [23:0] y;
  initial begin
    x = {<<8{a}};             $display("%h", x);
    x = {<<{a}};              $display("%h", x);
    x = {>>{a}};              $display("%h", x);
    $display("%b", 6'({<<4{b}}));
    $display("%b", 8'({<<2{c}}));
    y = {<<8{a, c}};          $display("%h", y);
    y = {<<byte{a}};          $display("%h", y);
    x = {<<4{c}};             $display("%h", x);
    $display("%h", 32'(unsigned'(1 ? {<<8{a}} : a)));
  end
endmodule
module t2;
  logic [7:0] p, q;
  logic [5:0] r;
  logic [3:0] u, v;
  initial begin
    {>>{p, q}} = 16'hABCD;            $display("%h %h", p, q);
    {<<8{p, q}} = 16'hABCD;           $display("%h %h", p, q);
    {<<{u}} = 4'b0001;                $display("%b", u);
    {<<4{r}} = 6'b010111;             $display("%b", r);
    {>>{u, v}} = 12'hABC;             $display("%h %h", u, v);
    {<<4{r}} = {<<4{6'b110101}};      $display("%b", r);
    {<<2{p}} <= 8'b0100_0011; #1      $display("%b", p);
  end
endmodule
