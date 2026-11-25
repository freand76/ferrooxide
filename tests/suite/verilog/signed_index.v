// expected
// A signed index narrower than 32 bits keeps its sign: -1 is out of range (x on read, no
// write), not the unsigned value of its bits.
module top;
  reg signed [7:0] arr [0:7];
  reg signed [2:0] a [0:3];
  reg signed [1:0] idx2;
  reg [1:0] uidx;
  reg [7:0] v;
  integer i;
  initial begin
    for (i = 0; i < 8; i = i + 1) arr[i] = i + 1;
    for (i = 0; i < 4; i = i + 1) a[i] = i;
    v = 8'hff;
    idx2 = -1;
    uidx = 2'b11;
    $display("%b %b %b %b", a[idx2], v[idx2], arr[0][idx2], arr[a[idx2]]);
    $display("%b %b %b", a[uidx], v[uidx], arr[0][uidx]);
    a[idx2] = 3'b101;
    v[idx2] = 1'b0;
    arr[1][idx2] = 1'b0;
    $display("%b %b %b", a[3], v, arr[1]);
    i = -1;
    $display("%b %b", a[i], v[i]);
  end
endmodule
