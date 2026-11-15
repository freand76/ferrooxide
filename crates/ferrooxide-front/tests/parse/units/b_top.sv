`default_nettype none
module top;
  import q::rec_t;
  rec_t r;
  word_t w2;
  initial begin
    r.w = `WIDTH'd5;
    w2 = q::N;
    counter = inc(counter);
  end
endmodule
