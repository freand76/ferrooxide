// expected
// streaming concatenations with queues and dynamic arrays (IEEE 1800-2017 11.4.14): packed
// into a queue (whole elements, the last filled up with 0 bits), unpacked with the first
// dynamic item taking what the fixed-size items after it leave, `with [r]` slices
module top;
  int h, l, h2, l3;
  byte c2;
  byte d[], d2[], d3[], d4[];
  byte q[$], q2[$], r[$];
  bit [3:0] n[$];
  logic [15:0] v16;
  logic [7:0] x8;
  initial begin
    h = 32'h0A0B0C0D;
    d = new[3];
    d[0] = 1; d[1] = 2; d[2] = 3;
    q = {>> byte {h, d}};
    $display("%0d: %p", q.size(), q);
    q2 = {<< byte {h, d}};
    $display("%0d: %p", q2.size(), q2);
    {>> byte {l, d2}} = q;
    $display("%h %0d: %p", l, d2.size(), d2);
    {<< 8 {h2, d3, c2}} = q2;
    $display("%h %0d: %p %0d", h2, d3.size(), d3, c2);
    r = {0, 0, 0, 2, 9, 8, 7};
    {>> byte {l3, d4 with [0 +: l3]}} = r;
    $display("%0d %0d: %p", l3, d4.size(), d4);
    v16 = {>> byte {d with [1:2]}};
    $display("%h", v16);
    n = {>> {8'hA5, 4'h3}};
    $display("%0d: %p", n.size(), n);
    r = {>> {12'hABC}};
    $display("%0d: %p", r.size(), r);
    x8 = {>> {d}};
    $display("%h", x8);
  end
endmodule
